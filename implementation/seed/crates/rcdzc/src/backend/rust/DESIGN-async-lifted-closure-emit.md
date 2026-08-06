# DESIGN — Rust `rust-async` backend: emit for lambda-lifted closures

**Status:** OPTION C LANDED (v-rust-backend, MR `c6f1f3f2`, 2026-07-20) — rust-async 3510→3869/0 (+359).
Option A (await-in-closure-body, boxed-future ABI + `DynCdzEnv` shim) remains. UNTRACKED scratch.

## ✅ Option C landed (call-free closure bodies)
`emit_lifted_lambda` emits a call-free async lifted body as a plain SYNC `fn` (forced `Mode::Sync` after
the guard); a body WITH a call still declines. Predicate: `layout::body_has_call` (reuses
`collect_call_callees`). SIMPLER than scoped — only 2 `is_async` sites in `expr.rs` (:236 decline, :1238
`Core::Call`), so `Core::Closure`/`CallClosure` (:2029/:2102) needed NO change. The bulk of the work was
two async GATE-HARNESS bugs in `xtask/main.rs` (factory-param off-by-one on `__cdz_env`; `call_or_await`
mangled the factory `export(caps)(applied)` two-group shape → new `split_factory_application`). See the
vertical log ([[backend-retarget-seam-and-rust-target]]) for the full landing + the masked-warning gate
FAIL debugging technique (`RCDZC_GATE_KEEP=1`).

## ⏭️ Option A remains (await-in-closure-body) — the ~188 still-todo rust-async cases

The original scoping (below) stands for the remaining boxed-future work.

## The gap (measured on trunk `4c2f1be3c`)
`rust-async` grades **3510 pass / 535 todo / 0 fail** vs sync `rust` **3888 / 157 / 0**. The delta is
concentrated in exactly two files, and BOTH are one root cause:

| file | rust pass | rust-async pass | gap | cause |
|------|-----------|-----------------|-----|-------|
| `21-host-closures.sexp` | 300 | 6 | **294** | async lifted-closure decline |
| `09-functions.sexp` | 321 | 239 | **82** | async lifted-closure decline (measured: ALL 82) |

Measured by compiling every 09-functions program on `--target rust-async`: of the 118 non-OK, **82 are
exactly `"an async lambda-lifted closure is not yet emitted by the Rust backend"`**; the other ~36 are
`CDZxxxx` front-end REJECT cases that are `todo` on every backend (not an emit gap — leave them). So
this ONE slice unlocks ~376 cases.

## Where it declines
`backend/rust/expr.rs:236`, in `emit_lifted_lambda`:
```rust
if mode.is_async() {
    return Err(Reject::decline("an async lambda-lifted closure is not yet emitted by the Rust backend"));
}
```
A lifted lambda only exists when a `(fn …)` could NOT be β-reduced — i.e. it flows to run time as a
runtime closure value (passed to a recursive fn, stored in a compound, or exported as a host-closure
factory result). The sync path below (:241–298) is complete: captures become ordinary LEADING params,
then the lambda's own params; `Core::Captured{index}` reads `__cap{index}`.

Three emit sites participate (all in `expr.rs`):
1. **`emit_lifted_lambda` (:226)** — emits `fn __lifted_{k}(<caps…>, <params…>) -> ret { body }`.
2. **`Core::Closure {code, captures}` (:2029)** — builds the runtime closure value:
   `{ let __c0 = <cap>; … std::rc::Rc::new(move |__a0,…| __lifted_k(__c0.clone(), …, __a0, …)) as
   std::rc::Rc<dyn Fn(A,…) -> R> }`.
3. **`Core::CallClosure {closure, args}` (:2102)** — `(<closure>)(<a0>, …)`.

## The async convention to mirror (`mod.rs:718–725`, `cdz-rt`)
A sync `fn f(…) -> R` becomes, in async mode:
```rust
async fn f<__CdzE: CdzEnv>(__cdz_env: &mut __CdzE, …) -> R { __cdz_env.consume(1).await; <body> }
```
and every emitted CALL becomes `Box::pin(callee(__cdz_env, …)).await`. `CdzEnv::consume` is RPITIT
(`fn consume(&mut self, gas: u64) -> impl Future<Output=()>`), no `async_trait` dep. Helpers:
`ENV_TYPE_PARAM` (`__CdzE`), `ENV_PARAM` (the value param name). The `CdzEnv` trait now lives in the
shared `cdz-rt` rlib (`use cdz_rt::CdzEnv;` preamble), NOT re-emitted per module.

## The design CRUX — an async closure cannot be `Rc<dyn Fn(A) -> R>`
Two hard facts collide:
- An `async fn` returns an opaque `impl Future`, not `R`. So the closure value's type is
  `Fn(A) -> <some Future of R>`, not `Fn(A) -> R`.
- The env is a GENERIC type param `<__CdzE: CdzEnv>`. A `dyn Fn` trait object CANNOT be generic over
  `__CdzE`, and it cannot borrow `&mut env` for the returned future's lifetime while staying `Fn`
  (not `FnMut`/`FnOnce`) and `Clone` (the sync path relies on `Rc<dyn Fn>` being callable repeatedly).

### Options
- **(A) Boxed-future, env-per-call closure ABI.** The runtime closure is
  `Rc<dyn Fn(&mut DynEnv, A) -> Pin<Box<dyn Future<Output=R> + '_>>>`, where the env is passed AT THE
  CALL (not captured). `Core::CallClosure` becomes
  `Box::pin((<closure>)(__cdz_env, <a0>,…)).await` — but the closure body itself calls the lifted
  `async fn __lifted_k(env, …)`, so the closure must forward `env` INTO the future. This needs a
  concrete `DynEnv` (a `dyn CdzEnv`-shaped object or a boxed `&mut dyn CdzEnv`), because `dyn Fn`
  can't be generic. Requires a small `cdz-rt` addition: an object-safe `DynCdzEnv` (a `dyn`-compatible
  facet of `CdzEnv`, since the RPITIT `consume` is NOT object-safe as-is → needs a
  `fn consume_boxed(&mut self, u64) -> Pin<Box<dyn Future<Output=()> + '_>>` shim). **Most faithful;
  most machinery.**
- **(B) Monomorphize the closure over the concrete gate env.** The gate links `cdz-rt` and drives with
  ONE concrete `CdzEnv` impl. If the closure type spells that concrete env, `Rc<dyn Fn(&mut ConcreteEnv,
  A) -> Pin<Box<dyn Future<Output=R>>>>` works with no object-safety shim. But it BAKES the gate's env
  type into emitted code — wrong for the "drop into any Rust codebase" goal; the closure ABI must be
  env-generic like every emitted `fn`. **Reject — violates the no-FFI/portable-crate invariant.**
- **(C) Sync-capture, sync-body closures only (partial slice).** Emit the async lifted closure ONLY
  when its body does NOT itself await (no recursive call / no runtime-op inside) — i.e. the body is
  effectively sync, so the closure stays `Rc<dyn Fn(A) -> R>` and only the ENCLOSING fn is async. This
  unlocks the SIMPLE host-closure cases (`(fn (x) (+ x 1))` — the bulk of 21-host-closures' 294) without
  the boxed-future machinery, and leaves an await-in-closure-body as a clean `todo`. **Smallest first
  slice; likely covers most of the 294 host-closure cases (their bodies are pure arithmetic).**

### Recommendation
Ship **(C) first** (a gated, sub-slice: async-enclosing + sync-closure-body → the existing `Rc<dyn Fn>`
emit, just allowed through when the body has no await), measure the unlock, THEN do **(A)** for the
await-in-body remainder with the `DynCdzEnv` object-safe shim in `cdz-rt`. (B) is rejected. If (C)'s
"body has no await" predicate is subtle (a closure body calling another lifted closure DOES await),
that's the boundary between the two slices — reuse `body_diverges`-style structural analysis or a new
`body_awaits(db, id)` walk (any `Core::Call`/`CallClosure`/runtime-op ⇒ awaits).

**OPEN QUESTION for the concierge if (C)'s predicate proves ambiguous or (A)'s `cdz-rt` shim needs a
frozen-hash/ABI sign-off:** does adding an object-safe `DynCdzEnv` facet to `cdz-rt` need coordination
with whoever owns the async runtime ABI? (`cdz-rt` is a shared rlib; a new pub trait is additive but
worth a heads-up.) — Send an `ask` only when actually blocked; the (C) sub-slice needs no `cdz-rt`
change, so start there.

## Gate plan
- Extend rust-async coverage: `21-host-closures.sexp --target rust-async` (expect ~+294 after A; +most
  after C), `09-functions.sexp --target rust-async` (+82 after A).
- Own the `.gate-baseline-rust-async` refresh — verify the diff is only genuine new-passes / sound
  declines (the deferred-baseline discipline: never adopt a masked fail).
- rcdzc unit tests: a lifted async closure emits `async fn __lifted_k<__CdzE: CdzEnv>(env, …)` + the
  call site awaits; rustc-roundtrip a host-closure factory on `--target rust-async`.

## Sequencing note
BLOCKED from starting until MR `e24642c64` (empty-set control-flow-join test pin) lands — the
per-commit cadence forbids a 2nd queued MR, and re-syncing would orphan that MR's `--ref`.

## ✅ Option A IMPLEMENTATION PLAN (2026-08-06, v-rust-backend — DynCdzEnv landed ae9faa12d/#2350)
Both gates cleared (concierge GO, v-runtime landed the object-safe shim). Confirmed shape in tree (cdz-rt/lib.rs:43): `trait DynCdzEnv { fn consume_boxed(&mut self, gas) -> Pin<Box<dyn Future<Output=()>+'_>> }` + `impl<E:CdzEnv> DynCdzEnv for E`.

**THE OBJECT-SAFETY RESOLUTION (the design crux, decided):** a closure-lifted async fn must take `env: &mut dyn DynCdzEnv` — NOT the generic `<__CdzE: CdzEnv>(env: &mut __CdzE)` a top-level async fn takes. Reason: the closure value is `Rc<dyn Fn(&mut dyn DynCdzEnv, A) -> Pin<Box<dyn Future<Output=R>+'_>>>` — the `dyn Fn` cannot be generic over `__CdzE`, so it holds a `&mut dyn DynCdzEnv` and must call the lifted fn through THAT. So the lifted fn's body charges gas via `env.consume_boxed(1).await` (the object-safe method) instead of `env.consume(1).await`. This means a closure-lifted async fn has a DISTINCT env-param type (`&mut dyn DynCdzEnv`) from a top-level async fn (`&mut __CdzE`) — thread a flag/param through `emit_lifted_lambda` + the async `Core::Call`/entry-gas sites to pick `consume_boxed` vs `consume` and `dyn DynCdzEnv` vs `__CdzE`.

**EDITS (one coherent MR, gate both targets green per step; call-free + all-sync stay byte-identical):**
1. `emit_lifted_lambda` (expr.rs:382): for `mode.is_async() && body_has_call`, DON'T decline — emit `async fn __lifted_k(__cdz_env: &mut dyn DynCdzEnv, caps…, params…) -> ret` and emit the body in Async mode (NOT forced Sync) so Core::Call threads env/awaits. Entry gas: `__cdz_env.consume_boxed(1).await`. (call-free async still emits as sync fn — keep that branch.)
2. `Core::Closure` (expr.rs:2446): when the lam is async+call-bearing, the closure value = `Rc::new(move |__cdz_env: &mut dyn DynCdzEnv, __a0,…| Box::pin(async move { __lifted_k(__cdz_env, __c0.clone(),…, __a0,…).await })) as Rc<dyn Fn(&mut dyn DynCdzEnv, A) -> Pin<Box<dyn Future<Output=R>+'_>>>`. (sync / call-free-async keep the plain `Rc<dyn Fn(A)->R>` form.)
3. `Core::CallClosure` (grep it): async+boxed-future closure → `Box::pin((<closure>)(__cdz_env, <args>)).await` (pass env, await the returned future). sync stays `(<closure>)(<args>)`.
4. `rust_type` / the `dyn_ty` render in Core::Closure: the async closure `Ty::Fn` must render the boxed-future form. Scope it to async mode (a Ty::Fn in sync mode stays `Rc<dyn Fn(A)->R>`). ⚠ rust_type is pure/no-Db/no-mode — so the boxed-future spelling likely lives at the Closure/CallClosure emit sites (which have ctx.mode), NOT in rust_type; keep rust_type's Ty::Fn = sync form, special-case async at the emit sites.
5. Gate-harness (xtask/main.rs): the async driver calls a closure now via `(clos)(&mut env, args).await` through `&mut dyn DynCdzEnv` — verify the harness passes a `&mut dyn DynCdzEnv` (coercion from its concrete env). May need a `&mut env as &mut dyn DynCdzEnv` at the call site.
VALIDATE: rustc-roundtrip a host-closure factory on --target rust-async (e.g. 21-host-closures cases); gate --check --target rust (must stay byte-identical, 0 newly/0 regress) + --target rust-async (expect +N newly-passing from the 148 todos); flip the unblocked -rust-async baseline rows; ping v-effects to build handle/resume arm on top.
