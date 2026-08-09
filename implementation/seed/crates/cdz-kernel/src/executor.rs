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

use crate::effect::{EffectId, EffectKind, EffectRequest, Payload};
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
    /// Perform `req` — the un-suffixed name (the whole trait is `async`, so an `_async` suffix would be
    /// redundant). Returns the outcome the kernel folds back as an `EffectResult`. May `.await` real I/O.
    ///
    /// `id` is the kernel `EffectId` the loop assigned this dispatch — the SAME id that keys the durable
    /// `Dispatched` frame and that a later [`crate::kernel::Session::settle_effect_result`] settles. Most
    /// executors ignore it (they answer synchronously and the loop folds the returned outcome). A DELEGATING
    /// executor NEEDS it: a userspace-effect executor that returns [`EffectOutcome::Deferred`] forwards the
    /// request to a handler session and must bind its reply-token to `(caller, id)` so the handler's
    /// `effect/reply` settles the RIGHT open effect (`settle_effect_result` takes the `EffectId`). The id is
    /// already on the log in the `Dispatched` record, so surfacing it here exposes no new kernel state.
    ///
    /// `idempotency_key` lets a side-effecting executor dedup a re-driven dispatch after a crash (a
    /// `Hash` = `idempotency_key_for(id, req)`, not the id itself — it is a stable dedup handle, not a
    /// reversible id, which is why the id is passed distinctly).
    async fn perform(
        &mut self,
        id: EffectId,
        req: &EffectRequest,
        idempotency_key: Hash,
    ) -> EffectOutcome;

    /// Does this executor serve effect `family`? The MECHANISM dimension the capability-manifest
    /// projection ([`crate::effect::project_manifest`]) probes over the canonical family set — "does the
    /// host serve family X" — to compute `Absent` vs granted/denied, so the kernel's inline
    /// `control/capabilities` answer needs it reachable through `&dyn Executor`, not just the concrete
    /// [`CompositeExecutor`]. Default `false` = FAIL-SAFE: an executor that doesn't override under-reports
    /// (the family reads `Absent`), never falsely claims to serve one. In practice the top-level drive-loop
    /// executor is a `CompositeExecutor` (which overrides to its `by_family` map), so the manifest is
    /// accurate there; a single-kind leaf executor overrides to its one family for when it's used bare as a
    /// `dyn Executor`.
    fn handles_family(&self, _family: &str) -> bool {
        false
    }
}

/// The by-kind effect router (§2 "the kernel routes, doesn't distinguish"): holds `Box<dyn Executor>`
/// per kind and routes `perform` to the registered executor, awaiting it. A session that emits more
/// than one effect kind (an agent doing both `Http` and `Model`) wires one of these; a native-async
/// executor (a real Bedrock/HTTP one that awaits its transport) registers directly.
///
/// A request whose kind has no registered executor returns an OBSERVABLE [`EffectOutcome::Err`] (the
/// reducer folds it — §9d anti-stuck: an unroutable effect is a normal failure event, not a panic or a
/// wedge), never a silent drop; `idempotency_key` passes through so a routed executor keeps its crash-dedup
/// contract (§16c-S1).
#[derive(Default)]
pub struct CompositeExecutor {
    /// Executors keyed by effect FAMILY STRING (seq-39), not the `EffectKind` enum — so a request routes
    /// by `req.content_type.family`, the same string authz/codec use. Registered via
    /// [`with_effect`](Self::with_effect) (register-by-string — a NEW family is served with no `EffectKind`
    /// variant + no kernel edit).
    by_family: HashMap<String, Box<dyn Executor>>,
}

impl CompositeExecutor {
    pub fn new() -> Self {
        CompositeExecutor {
            by_family: HashMap::new(),
        }
    }

    /// Register an effect executor for a FAMILY STRING (register-by-string): a new effect type is served
    /// by registering a handler for its family, with NO [`EffectKind`] variant + no kernel recompile.
    /// Builder-style; last registration wins. Keyed by the family string the router matches
    /// (`req.content_type.family`). The canonical registration API — callers pass a family const
    /// ([`crate::effect::effect_ct`]) or any family string. (control/* families register their in-kernel/
    /// host-surfaced dispositions in a later beat; this registers an executor-routed effect/* family.)
    pub fn with_effect(mut self, family: impl Into<String>, executor: Box<dyn Executor>) -> Self {
        self.by_family.insert(family.into(), executor);
        self
    }

    /// Is an executor registered for this kind (i.e. for its family)?
    pub fn handles(&self, kind: &EffectKind) -> bool {
        self.handles_family(kind.family())
    }

    /// Is an executor registered for this effect FAMILY string? INHERENT method — the registration source
    /// of truth (`by_family`), callable without the [`Executor`] trait in scope. The trait impl's
    /// `handles_family` override delegates here, so `&dyn Executor` sees the same answer as a concrete
    /// `CompositeExecutor` caller (one source of truth, two reach-paths).
    pub fn handles_family(&self, family: &str) -> bool {
        self.by_family.contains_key(family)
    }
}

#[async_trait::async_trait(?Send)]
impl Executor for CompositeExecutor {
    async fn perform(
        &mut self,
        id: EffectId,
        req: &EffectRequest,
        idempotency_key: Hash,
    ) -> EffectOutcome {
        // Route by the request's content-type FAMILY (seq-39): a request whose family has no registered
        // executor is an OBSERVABLE Err (§9d anti-stuck), never a panic/drop. `id` threads through unchanged
        // so a delegating leaf executor can bind its reply-token to (caller, id).
        match self.by_family.get_mut(req.content_type.family.as_ref()) {
            Some(inner) => inner.perform(id, req, idempotency_key).await,
            None => EffectOutcome::err(format!(
                "no executor registered for effect family {:?} (target {:?})",
                req.content_type.family, req.target
            )),
        }
    }

    /// The composite serves exactly the families it has a registered executor for. Overrides the trait's
    /// fail-safe `false` default by delegating to the inherent [`CompositeExecutor::handles_family`] (the
    /// `by_family` registration source of truth) — so `&dyn Executor` (the drive loop's inline
    /// capability-manifest projection) gets the accurate answer, matching a concrete caller. Since the
    /// top-level drive-loop executor is in practice a `CompositeExecutor`, the projection is accurate
    /// without needing every leaf executor to override.
    fn handles_family(&self, family: &str) -> bool {
        CompositeExecutor::handles_family(self, family)
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

#[async_trait::async_trait(?Send)]
impl Executor for RecordingExecutor {
    async fn perform(
        &mut self,
        _id: EffectId,
        req: &EffectRequest,
        idempotency_key: Hash,
    ) -> EffectOutcome {
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
/// **No shell = no injection (PR#992 WARNING:WARNING: command injection):** the previous `sh -c <target>` let shell
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
    async fn perform(
        &mut self,
        _id: EffectId,
        req: &EffectRequest,
        _idempotency_key: Hash,
    ) -> EffectOutcome {
        use crate::effect::EffectKind;
        if req.kind != EffectKind::Shell {
            return EffectOutcome::err(format!(
                "ShellExecutor only handles Shell effects, got {:?}",
                req.kind
            ));
        }
        // Split the target into program + args on whitespace and exec DIRECTLY — no shell, so
        // metacharacters are literal arguments, not interpreted (PR#992 CWE-78 fix).
        let mut parts = req.target.split_whitespace();
        let Some(program) = parts.next() else {
            return EffectOutcome::err("empty command".to_string());
        };
        let args: Vec<&str> = parts.collect();
        let output = std::process::Command::new(program).args(&args).output();
        match output {
            Ok(out) if out.status.success() => {
                EffectOutcome::Ok(Some(Payload::Inline(out.stdout.into())))
            }
            Ok(out) => EffectOutcome::err(format!(
                "exit {}: {}",
                out.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&out.stderr).trim()
            )),
            Err(e) => EffectOutcome::err(format!("spawn failed: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::EffectKind;

    fn req(kind: EffectKind, target: &str) -> EffectRequest {
        EffectRequest::new(kind, target, None, crate::effect::Timeliness::Interactive)
    }

    // A test executor that tags its Ok payload so we can prove WHICH inner executor ran. Native async.
    struct TagExecutor(&'static [u8]);
    #[async_trait::async_trait(?Send)]
    impl Executor for TagExecutor {
        async fn perform(
            &mut self,
            _id: EffectId,
            _req: &EffectRequest,
            _key: Hash,
        ) -> EffectOutcome {
            EffectOutcome::Ok(Some(Payload::Inline(self.0.to_vec().into())))
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tag_executor_is_object_safe_as_dyn_executor() {
        // Drivable through &mut dyn Executor — object-safety (the reason for async-trait).
        let mut tagged = TagExecutor(b"async-ran");
        let dyn_exec: &mut dyn Executor = &mut tagged;
        let out = dyn_exec
            .perform(
                EffectId(1),
                &req(EffectKind::Http, "https://ok/x"),
                Hash::of(b"k"),
            )
            .await;
        assert_eq!(
            out,
            EffectOutcome::Ok(Some(Payload::Inline(b"async-ran".to_vec().into())))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn composite_routes_each_kind_to_its_executor() {
        let mut exec = CompositeExecutor::new()
            .with_effect(
                crate::effect::effect_ct::HTTP,
                Box::new(TagExecutor(b"http-ran")),
            )
            .with_effect(
                crate::effect::effect_ct::SHELL,
                Box::new(TagExecutor(b"shell-ran")),
            );
        assert!(exec.handles(&EffectKind::Http));
        assert!(exec.handles(&EffectKind::Shell));

        // Each kind reaches its own executor — the multi-kind session a single executor couldn't serve.
        assert_eq!(
            exec.perform(
                EffectId(1),
                &req(EffectKind::Http, "https://ok/x"),
                Hash::of(b"k")
            )
            .await,
            EffectOutcome::Ok(Some(Payload::Inline(b"http-ran".to_vec().into())))
        );
        assert_eq!(
            exec.perform(
                EffectId(1),
                &req(EffectKind::Shell, "echo hi"),
                Hash::of(b"k")
            )
            .await,
            EffectOutcome::Ok(Some(Payload::Inline(b"shell-ran".to_vec().into())))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn composite_unroutable_kind_is_an_observable_err_not_a_drop() {
        // A kind with no registered executor → Err (an observable outcome the reducer folds, §9d), never
        // a silent drop or panic.
        let mut exec = CompositeExecutor::new()
            .with_effect(crate::effect::effect_ct::HTTP, Box::new(TagExecutor(b"h")));
        assert!(!exec.handles(&EffectKind::Model));
        match exec
            .perform(EffectId(1), &req(EffectKind::Model, "gpt"), Hash::of(b"k"))
            .await
        {
            EffectOutcome::Err { message: msg, .. } => {
                // The message names the unroutable FAMILY (seq-39: routing keys on the family string).
                assert!(
                    msg.contains(EffectKind::Model.family()),
                    "err names the unroutable family: {msg}"
                );
            }
            other => panic!("expected Err for an unroutable kind, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn with_effect_registers_by_family_string_including_an_extension_family() {
        // register-by-string: with_effect keys on a FAMILY STRING, so a NEW effect family with NO
        // EffectKind variant is servable + routable. And `with(EffectKind)` delegates to it (same result).
        let mut exec = CompositeExecutor::new()
            // A well-known family via the const (what the host migrates to)...
            .with_effect(
                crate::effect::effect_ct::HTTP,
                Box::new(TagExecutor(b"http-e")),
            )
            // ...and an EXTENSION family with no EffectKind variant — the whole point of register-by-string.
            .with_effect("embedding", Box::new(TagExecutor(b"embed-e")));
        assert!(exec.handles_family(crate::effect::effect_ct::HTTP));
        assert!(exec.handles_family("embedding"));
        // Route a request whose content_type.family is the extension family → its executor runs.
        let mut ext = req(EffectKind::Http, "x");
        ext.content_type.family = "embedding".into();
        assert_eq!(
            exec.perform(EffectId(1), &ext, Hash::of(b"k")).await,
            EffectOutcome::Ok(Some(Payload::Inline(b"embed-e".to_vec().into())))
        );
        // with(EffectKind) delegates to with_effect(kind.family(), ..) — same registration.
        let mut viaenum = CompositeExecutor::new()
            .with_effect(crate::effect::effect_ct::HTTP, Box::new(TagExecutor(b"h")));
        assert!(viaenum.handles_family(crate::effect::effect_ct::HTTP));
        assert_eq!(
            viaenum
                .perform(EffectId(1), &req(EffectKind::Http, "x"), Hash::of(b"k"))
                .await,
            EffectOutcome::Ok(Some(Payload::Inline(b"h".to_vec().into())))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn composite_last_registration_wins() {
        // Registering a kind twice replaces the prior executor (deliberate override).
        let mut exec = CompositeExecutor::new()
            .with_effect(
                crate::effect::effect_ct::HTTP,
                Box::new(TagExecutor(b"first")),
            )
            .with_effect(
                crate::effect::effect_ct::HTTP,
                Box::new(TagExecutor(b"second")),
            );
        assert_eq!(
            exec.perform(EffectId(1), &req(EffectKind::Http, "x"), Hash::of(b"k"))
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
            async fn perform(
                &mut self,
                _id: EffectId,
                _req: &EffectRequest,
                key: Hash,
            ) -> EffectOutcome {
                EffectOutcome::Ok(Some(Payload::Inline(key.as_bytes().to_vec().into())))
            }
        }
        let mut exec =
            CompositeExecutor::new().with_effect(crate::effect::effect_ct::HTTP, Box::new(KeyEcho));
        let key = Hash::of(b"the-key");
        assert_eq!(
            exec.perform(EffectId(1), &req(EffectKind::Http, "x"), key)
                .await,
            EffectOutcome::Ok(Some(Payload::Inline(key.as_bytes().to_vec().into())))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn composite_threads_the_effect_id_to_the_inner_executor() {
        // userspace-effects I3: the delegating executor binds its reply-token to the dispatch's EffectId, so
        // the router MUST forward the SAME id (the one keying the open Dispatched frame) to the inner leaf
        // executor unchanged — else a Deferred executor would mint a token against the wrong id and its
        // later `effect/reply` would settle the wrong open effect. Prove the id round-trips through the route.
        struct IdEcho;
        #[async_trait::async_trait(?Send)]
        impl Executor for IdEcho {
            async fn perform(
                &mut self,
                id: EffectId,
                _req: &EffectRequest,
                _key: Hash,
            ) -> EffectOutcome {
                EffectOutcome::Ok(Some(Payload::Inline(id.0.to_le_bytes().to_vec().into())))
            }
        }
        let mut exec =
            CompositeExecutor::new().with_effect(crate::effect::effect_ct::HTTP, Box::new(IdEcho));
        assert_eq!(
            exec.perform(EffectId(4242), &req(EffectKind::Http, "x"), Hash::of(b"k"))
                .await,
            EffectOutcome::Ok(Some(Payload::Inline(4242u64.to_le_bytes().to_vec().into()))),
            "the composite router forwards the dispatch EffectId to the inner executor unchanged"
        );
    }

    #[test]
    fn handles_family_reports_the_registered_family_set_by_string() {
        // I2: the read-only mechanism accessor the manifest projection probes. A registered kind's family
        // is handled (by string); an unregistered family — including an extension family with no EffectKind
        // variant — is not. handles(&EffectKind) agrees with handles_family(kind.family()).
        let exec = CompositeExecutor::new()
            .with_effect(crate::effect::effect_ct::HTTP, Box::new(TagExecutor(b"h")));
        assert!(exec.handles_family(EffectKind::Http.family()));
        assert!(exec.handles(&EffectKind::Http)); // the enum sibling agrees
        assert!(!exec.handles_family(EffectKind::Model.family()));
        assert!(!exec.handles(&EffectKind::Model));
        // An extension family (no EffectKind variant) is cleanly "not handled", never a panic.
        assert!(!exec.handles_family("embedding"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn routing_keys_on_content_type_family_not_the_kind_enum() {
        // The extensible-effects invariant (seq-39): the router keys on `content_type.family`, NOT the
        // `EffectKind` enum. Prove it by DIVORCING the two — build a request whose `kind` is Http but whose
        // `content_type.family` is "model", and show it routes to the MODEL-registered executor. (Via `new`
        // the two agree by construction; the register-by-string slice will let a family exist with no
        // matching `EffectKind` at all, and this is the seam that makes that work — so pin it now.)
        let mut exec = CompositeExecutor::new()
            .with_effect(
                crate::effect::effect_ct::HTTP,
                Box::new(TagExecutor(b"http-executor")),
            )
            .with_effect(
                crate::effect::effect_ct::MODEL,
                Box::new(TagExecutor(b"model-executor")),
            );
        let mut r = req(EffectKind::Http, "x");
        r.content_type.family = EffectKind::Model.family().into();
        // Routed by family ("model") to the MODEL executor, despite kind == Http.
        assert_eq!(
            exec.perform(EffectId(1), &r, Hash::of(b"k")).await,
            EffectOutcome::Ok(Some(Payload::Inline(b"model-executor".to_vec().into())))
        );
        // And a family with NO registered executor (and no EffectKind variant) is an observable Err naming
        // that family — the fail-closed seam the register-by-string slice hardens to retry::permanent.
        let mut ext = req(EffectKind::Http, "x");
        ext.content_type.family = "embedding".into();
        match exec.perform(EffectId(1), &ext, Hash::of(b"k")).await {
            EffectOutcome::Err { message: msg, .. } => assert!(
                msg.contains("embedding"),
                "unroutable extension family named in the err: {msg}"
            ),
            other => panic!("an unregistered family must be an observable Err, got {other:?}"),
        }
    }
}
