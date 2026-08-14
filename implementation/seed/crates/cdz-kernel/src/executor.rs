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

/// The effect router (§2 "the kernel routes, doesn't distinguish"): holds `Box<dyn Executor>` keyed by the
/// effect's SCHEMA-HASH identity (phase-3 re-key) and routes `perform` to the registered executor, awaiting
/// it. A session that emits more than one effect (an agent doing both `Http` and `Model`) wires one of these;
/// a native-async executor (a real Bedrock/HTTP one that awaits its transport) registers directly.
///
/// A request whose identity has no registered executor returns an OBSERVABLE [`EffectOutcome::Err`] (the
/// reducer folds it — §9d anti-stuck: an unroutable effect is a normal failure event, not a panic or a
/// wedge), never a silent drop; `idempotency_key` passes through so a routed executor keeps its crash-dedup
/// contract (§16c-S1).
#[derive(Default)]
pub struct CompositeExecutor {
    /// Executors keyed by their effect's SCHEMA-HASH (phase-3 identity re-key): the router resolves a
    /// request's schema-hash — its baked [`EffectRequest::schema_hash`] when present (the authoritative wire
    /// identity), else the family-derived [`effect_family_schema_hash`](crate::ast_marshal::effect_family_schema_hash)
    /// — and routes to the executor registered under that hash. [`with_effect`](Self::with_effect) computes
    /// the family's schema-hash at registration, so a well-known/declared family (which has one) is served by
    /// SCHEMA-HASH identity, not the family string. Behavior-equivalent to the old family-string routing
    /// because `req.schema_hash == effect_family_schema_hash(family)` for every in-host request (the identity
    /// `new_with_family` computes), so the same executor is selected.
    by_schema_hash: HashMap<Hash, Box<dyn Executor>>,
    /// Executors for schema-hash-LESS register-by-string families — a userspace extension family with no
    /// declared schema, so [`effect_family_schema_hash`](crate::ast_marshal::effect_family_schema_hash) is
    /// `None` and there is no hash to key on. These stay keyed by the family STRING. The well-known/declared
    /// families all carry a schema-hash and live in [`by_schema_hash`](Self::by_schema_hash); this map holds
    /// only the hashless remainder (register-by-string extension families) until phase-3's mandatory
    /// schema-hash flip retires that path.
    by_family: HashMap<String, Box<dyn Executor>>,
    /// Optional DEFAULT-ROUTE executor, consulted ONLY when [`by_family`](Self::by_family) has no EXACT
    /// match for the request's family (userspace-effects I3). The `by_family` map serves a FIXED,
    /// registered-ahead-of-time family set; a userspace-effect family is DYNAMIC — a handler session claims
    /// `effect/weather` / `effect/vector-search` / … at runtime, so there is no fixed string to
    /// [`with_effect`]-register a delegating executor under, and the map can't hold an open set. The
    /// fallback closes that gap: the host registers ONE `UserspaceEffectExecutor` here
    /// ([`with_fallback`](Self::with_fallback)) and it serves ANY family that resolves to a registered
    /// handler. `None` = no fallback (the pre-I3 behavior: an unmatched family is the no-executor Err).
    ///
    /// The fallback MUST self-guard (§9d anti-stuck preserved): it returns its OWN observable `Err` for a
    /// family it does not actually handle (e.g. no handler registered for it), so a genuinely-unroutable
    /// family still produces an observable failure the reducer folds — just via the fallback rather than
    /// the bare None arm. Routing NEVER silently drops.
    fallback: Option<Box<dyn Executor>>,
}

impl CompositeExecutor {
    pub fn new() -> Self {
        CompositeExecutor {
            by_schema_hash: HashMap::new(),
            by_family: HashMap::new(),
            fallback: None,
        }
    }

    /// Register an effect executor for a FAMILY STRING (register-by-string): a new effect type is served
    /// by registering a handler for its family, with NO [`EffectKind`] variant + no kernel recompile.
    /// Builder-style; last registration wins. Keyed by the family string the router matches
    /// (`req.content_type.family`). The canonical registration API — callers pass a family const
    /// ([`crate::effect::effect_ct`]) or any family string. (control/* families register their in-kernel/
    /// host-surfaced dispositions in a later beat; this registers an executor-routed effect/* family.)
    pub fn with_effect(mut self, family: impl Into<String>, executor: Box<dyn Executor>) -> Self {
        // Phase-3 re-key: key the executor by its effect's SCHEMA-HASH when the family has one (the
        // well-known/declared families), else by the family STRING (a schema-hash-less register-by-string
        // extension family). Route resolution in `perform`/`handles_family` mirrors this split.
        let family = family.into();
        match crate::ast_marshal::effect_family_schema_hash(&family) {
            Some(h) => {
                self.by_schema_hash.insert(h, executor);
            }
            None => {
                self.by_family.insert(family, executor);
            }
        }
        self
    }

    /// Register the DEFAULT-ROUTE executor consulted when no [`with_effect`](Self::with_effect) family
    /// matches (userspace-effects I3 — see the [`fallback`](Self::fallback) field). Builder-style; last
    /// registration wins. The host registers a `UserspaceEffectExecutor` here so a DYNAMIC userspace-effect
    /// family (claimed at runtime, so not in the fixed `by_family` set) still routes to the delegating
    /// executor instead of hitting the no-executor Err. The fallback self-guards, so registering it does
    /// NOT make every unknown family "handled" — a family it can't serve still returns an observable Err.
    pub fn with_fallback(mut self, executor: Box<dyn Executor>) -> Self {
        self.fallback = Some(executor);
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
        // A family is handled if an exact executor is registered OR the fallback serves it. The fallback's
        // `handles_family` is its own honest answer (a `UserspaceEffectExecutor` reports true only for a
        // family that resolves to a registered handler), so the manifest projection stays accurate — it
        // does NOT blanket-claim every family just because a fallback exists.
        let registered = match crate::ast_marshal::effect_family_schema_hash(family) {
            // A family with a schema-hash is registered under that hash (phase-3 re-key)...
            Some(h) => self.by_schema_hash.contains_key(&h),
            // ...a schema-hash-less register-by-string extension family stays keyed by the family string.
            None => self.by_family.contains_key(family),
        };
        registered
            || self
                .fallback
                .as_ref()
                .is_some_and(|f| f.handles_family(family))
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
        // Route by the effect's SCHEMA-HASH identity (phase-3 re-key): the request's baked `schema_hash` is
        // authoritative, else the family-derived hash. `id` threads through unchanged so a delegating leaf
        // executor can bind its reply-token to (caller, id).
        let route_hash = req.schema_hash.or_else(|| {
            crate::ast_marshal::effect_family_schema_hash(req.content_type.family.as_ref())
        });
        if let Some(h) = route_hash {
            if let Some(inner) = self.by_schema_hash.get_mut(&h) {
                return inner.perform(id, req, idempotency_key).await;
            }
        }
        // No schema-hash executor: a schema-hash-less register-by-string extension family routes by string.
        match self.by_family.get_mut(req.content_type.family.as_ref()) {
            Some(inner) => inner.perform(id, req, idempotency_key).await,
            // No exact match: consult the DEFAULT-ROUTE fallback (userspace-effects I3) if registered — it
            // serves DYNAMIC families (a userspace effect claimed at runtime, absent from the fixed set) and
            // self-guards (returns its own Err for a family it can't handle). With no fallback this is the
            // original no-executor OBSERVABLE Err (§9d anti-stuck), never a drop/panic.
            None => match self.fallback.as_mut() {
                Some(fb) => fb.perform(id, req, idempotency_key).await,
                None => EffectOutcome::err(format!(
                    "no executor registered for effect family {:?} (target {:?})",
                    req.content_type.family, req.target
                )),
            },
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
/// world. Runs an `EffectKind::Shell` request whose PAYLOAD carries a STRUCTURED command — a
/// `(shell-pipeline (stage (program …) (args …)))` (see [`crate::event_ast::ShellPipeline`]) — executed
/// **directly via `Command::new(program).args(args)` — NO `sh -c`, NO whitespace-splitting** (PR#992
/// security fix, CWE-78 + operator structured-args directive). Returns the outcome the kernel folds back:
/// - exit 0 → `Ok(Some(stdout))`
/// - non-zero exit → `Err("exit <code>: <stderr>")`
/// - spawn failure / empty|malformed command → `Err`.
///
/// **Structured args, never a split (operator directive):** the command is a `program` + a `Vec<arg>`,
/// each arg a LITERAL string, exactly like `std::process::Command::new(program).args(vec)`. This replaced
/// the old brittle model (a flat `target` string `split_whitespace()`'d into program+args), which broke on
/// quoted args, paths/args containing spaces, and empty args (a `"foo bar"` arg wrongly became two). The
/// structured payload carries args explicitly, so an arg with spaces survives and NOTHING is re-split.
///
/// **No shell = no injection (PR#992):** direct exec makes every token a LITERAL argument — a `;`/`|`/`$()`
/// is an argument to the program, not a shell separator — so the structural defense holds regardless of
/// the SEC-F1 allow-list being metachar-proof. Structured args STRENGTHEN this: there is no string to
/// mis-split, so the metacharacter/splitting surface is closed at the model, not just the exec call.
///
/// **Single command = a one-stage pipeline:** this single-process executor runs a pipeline of exactly ONE
/// stage (the structured successor to the old bare target). A multi-stage pipeline (`a | b`) is the host's
/// piped-stdio spawner (each stage its own SEC-F1-gated program); this kernel executor rejects a >1-stage
/// pipeline as out-of-scope for the single-process path.
///
/// **Trust boundary:** does NOT re-authorize; the kernel gated the resolved `program` (the effect target)
/// against a resource-scoped capability (SEC-F1) before dispatch.
///
/// **Non-Shell effects** return an `Err` (single-KIND executor; register under `Shell` in a
/// [`CompositeExecutor`]). **Idempotency (§16c-S1):** a command is NOT generally idempotent; a real
/// deployment dedups on `idempotency_key`. Native async (the subprocess spawn is sync `std::process` today).
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
        // The shell command is a STRUCTURED {program, args} payload (operator directive: NO flat-string
        // whitespace split). Decode the `(shell-pipeline (stage (program) (args …)))` payload; each arg is a
        // literal string, so an arg with spaces survives and nothing is re-split.
        let payload_bytes = match &req.payload {
            Some(Payload::Inline(b)) => b.as_ref(),
            _ => return EffectOutcome::err(
                "shell effect has no structured command payload (expected a (shell-pipeline …))"
                    .to_string(),
            ),
        };
        let pipeline = match crate::event_ast::decode_shell_pipeline(payload_bytes) {
            Ok(p) => p,
            Err(e) => return EffectOutcome::err(format!("shell command payload malformed: {e:?}")),
        };
        // This single-process executor runs a ONE-stage pipeline (the structured successor to a bare
        // command). A multi-stage pipeline is the host's piped-stdio spawner, out of scope here.
        let stage = match pipeline.stages.as_slice() {
            [stage] => stage,
            [] => return EffectOutcome::err("empty command (no pipeline stage)".to_string()),
            _ => {
                return EffectOutcome::err(
                    "multi-stage shell pipeline not supported by the single-process executor"
                        .to_string(),
                )
            }
        };
        let program = stage.program.as_str();
        if program.is_empty() {
            return EffectOutcome::err("empty command".to_string());
        }
        let args: Vec<&str> = stage.args.iter().map(|a| a.as_str()).collect();
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
        // Route a request for the extension family → its executor runs. Built via new_with_family so the
        // request's schema_hash is CONSISTENT with the family (None for a schema-hash-less extension) — how a
        // real request is constructed; a hashless extension routes by its family string.
        let ext = crate::effect::EffectRequest::new_with_family(
            "embedding",
            "x",
            None,
            crate::effect::Timeliness::Interactive,
        );
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
    async fn routing_keys_on_the_schema_hash_identity_not_the_kind_enum() {
        // Phase-3 identity re-key: the router keys on the effect's SCHEMA-HASH (the request's `schema_hash`,
        // else the family-derived one), NOT the `EffectKind` enum. A request built via new_with_family("model")
        // carries the "model" schema-hash and routes to the MODEL-registered executor. (For a real request the
        // family and schema-hash agree by construction; the router never consults the kind enum.)
        let mut exec = CompositeExecutor::new()
            .with_effect(
                crate::effect::effect_ct::HTTP,
                Box::new(TagExecutor(b"http-executor")),
            )
            .with_effect(
                crate::effect::effect_ct::MODEL,
                Box::new(TagExecutor(b"model-executor")),
            );
        let r = crate::effect::EffectRequest::new_with_family(
            crate::effect::effect_ct::MODEL,
            "x",
            None,
            crate::effect::Timeliness::Interactive,
        );
        // Routed by the "model" schema-hash to the MODEL executor.
        assert_eq!(
            exec.perform(EffectId(1), &r, Hash::of(b"k")).await,
            EffectOutcome::Ok(Some(Payload::Inline(b"model-executor".to_vec().into())))
        );
        // And a family with NO registered executor (a schema-hash-less extension) is an observable Err naming
        // that family — the fail-closed seam the register-by-string slice hardens to retry::permanent.
        let ext = crate::effect::EffectRequest::new_with_family(
            "embedding",
            "x",
            None,
            crate::effect::Timeliness::Interactive,
        );
        match exec.perform(EffectId(1), &ext, Hash::of(b"k")).await {
            EffectOutcome::Err { message: msg, .. } => assert!(
                msg.contains("embedding"),
                "unroutable extension family named in the err: {msg}"
            ),
            other => panic!("an unregistered family must be an observable Err, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn routing_honors_a_baked_schema_hash_over_the_family() {
        // Phase-3 input-wire (0dc861710): a request MAY carry a PRODUCER-BAKED schema_hash that is the
        // AUTHORITATIVE wire identity. The re-keyed router keys on it — a request whose family string is
        // "http" but whose baked schema_hash is model's routes to the MODEL executor (identity = the
        // schema-hash, not the family string). This is the whole point of the schema-hash identity model.
        let mut exec = CompositeExecutor::new()
            .with_effect(
                crate::effect::effect_ct::HTTP,
                Box::new(TagExecutor(b"http")),
            )
            .with_effect(
                crate::effect::effect_ct::MODEL,
                Box::new(TagExecutor(b"model")),
            );
        let mut r = crate::effect::EffectRequest::new_with_family(
            crate::effect::effect_ct::HTTP,
            "x",
            None,
            crate::effect::Timeliness::Interactive,
        );
        // Override with the model schema-hash (as a phase-3 producer-baked identity would).
        r.schema_hash =
            crate::ast_marshal::effect_family_schema_hash(crate::effect::effect_ct::MODEL);
        assert_eq!(
            exec.perform(EffectId(1), &r, Hash::of(b"k")).await,
            EffectOutcome::Ok(Some(Payload::Inline(b"model".to_vec().into()))),
            "the baked schema_hash is the routing identity, overriding the family string"
        );
    }

    // A stand-in for the host's `UserspaceEffectExecutor` (userspace-effects I3): serves ONLY families it
    // "resolves a handler for" (here, a fixed allow-set), self-guarding by returning a PERMANENT Err for
    // any other family — exactly the anti-stuck contract the fallback relies on.
    struct FallbackExecutor {
        handled: &'static [&'static str],
    }
    #[async_trait::async_trait(?Send)]
    impl Executor for FallbackExecutor {
        async fn perform(
            &mut self,
            _id: EffectId,
            req: &EffectRequest,
            _key: Hash,
        ) -> EffectOutcome {
            if self.handled.contains(&req.content_type.family.as_ref()) {
                EffectOutcome::Ok(Some(Payload::Inline(b"fallback-ran".to_vec().into())))
            } else {
                // SELF-GUARD: a family this delegating executor does not actually handle still gets an
                // observable Err (§9d) — the fallback never blanket-accepts.
                EffectOutcome::err(format!(
                    "no handler registered for userspace effect family {:?}",
                    req.content_type.family
                ))
            }
        }
        fn handles_family(&self, family: &str) -> bool {
            self.handled.contains(&family)
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fallback_serves_a_dynamic_family_with_no_exact_registration() {
        // userspace-effects I3: a DYNAMIC family (no `with_effect` registration) routes to the fallback
        // instead of hitting the no-executor Err — the whole point of the default-route arm.
        let mut exec = CompositeExecutor::new()
            .with_effect(
                crate::effect::effect_ct::HTTP,
                Box::new(TagExecutor(b"http")),
            )
            .with_fallback(Box::new(FallbackExecutor {
                handled: &["effect/weather"],
            }));
        let ext = crate::effect::EffectRequest::new_with_family(
            "effect/weather",
            "today",
            None,
            crate::effect::Timeliness::Interactive,
        );
        assert_eq!(
            exec.perform(EffectId(1), &ext, Hash::of(b"k")).await,
            EffectOutcome::Ok(Some(Payload::Inline(b"fallback-ran".to_vec().into()))),
            "a dynamic family with no exact executor routes to the fallback"
        );
        // handles_family ORs the fallback's honest answer.
        assert!(exec.handles_family("effect/weather"));
        assert!(exec.handles_family(crate::effect::effect_ct::HTTP));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn exact_family_match_wins_over_the_fallback() {
        // The fallback is consulted ONLY on an exact-match miss — a registered family never reaches it.
        let mut exec = CompositeExecutor::new()
            .with_effect(
                crate::effect::effect_ct::HTTP,
                Box::new(TagExecutor(b"exact")),
            )
            .with_fallback(Box::new(FallbackExecutor {
                handled: &[crate::effect::effect_ct::HTTP], // even if the fallback claims it...
            }));
        assert_eq!(
            exec.perform(EffectId(1), &req(EffectKind::Http, "x"), Hash::of(b"k"))
                .await,
            EffectOutcome::Ok(Some(Payload::Inline(b"exact".to_vec().into()))),
            "an exact by_family match takes precedence over the fallback"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fallback_self_guard_preserves_the_anti_stuck_err_for_a_genuinely_unhandled_family() {
        // §9d anti-stuck preserved: with a fallback registered, a family NEITHER exactly-registered NOR
        // served by the fallback still produces an observable Err (from the fallback's self-guard) — routing
        // never silently drops just because a fallback exists.
        let mut exec = CompositeExecutor::new().with_fallback(Box::new(FallbackExecutor {
            handled: &["effect/weather"],
        }));
        let ext = crate::effect::EffectRequest::new_with_family(
            "effect/unregistered",
            "x",
            None,
            crate::effect::Timeliness::Interactive,
        );
        match exec.perform(EffectId(1), &ext, Hash::of(b"k")).await {
            EffectOutcome::Err { message: msg, .. } => assert!(
                msg.contains("effect/unregistered"),
                "the fallback's self-guard names the unhandled family: {msg}"
            ),
            other => panic!("a family the fallback can't handle must still Err, got {other:?}"),
        }
        // And handles_family is honest: false for a family neither path serves.
        assert!(!exec.handles_family("effect/unregistered"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn no_fallback_keeps_the_original_no_executor_err() {
        // Backward-compat: with NO fallback registered, an unmatched family is the original no-executor Err
        // (the pre-I3 behavior) — the fallback is purely additive.
        let mut exec = CompositeExecutor::new()
            .with_effect(crate::effect::effect_ct::HTTP, Box::new(TagExecutor(b"h")));
        let ext = crate::effect::EffectRequest::new_with_family(
            "effect/weather",
            "x",
            None,
            crate::effect::Timeliness::Interactive,
        );
        match exec.perform(EffectId(1), &ext, Hash::of(b"k")).await {
            EffectOutcome::Err { message: msg, .. } => {
                assert!(msg.contains("no executor registered"), "{msg}")
            }
            other => panic!("no fallback → the no-executor Err, got {other:?}"),
        }
        assert!(!exec.handles_family("effect/weather"));
    }
}
