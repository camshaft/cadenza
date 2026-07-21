# DESIGN — Rust backend host-closure export ABI (the ~129-todo closure family)

**Status:** scoping (v-rust-backend, tick-112/113). The single largest remaining rust-target decline
family. UNTRACKED scratch — commit once S1 lands (mirrors DESIGN-rust-runtime-trait.md convention).

## The gap
A CAPTURING closure crossing the export boundary declines at `mod.rs:437` (`emit_signature`):
`if public && (params.any(is_fn_ty) || is_fn_ty(result))` → "a function-typed value cannot cross the
Rust export boundary (no closure handle ABI)". A NON-capturing closure body already emits fine; an
INTERNAL closure (passed to a helper, never crossing an export edge) is unaffected. Only the EXPORT of a
function-typed value declines.

Two export shapes carry a closure:
1. **Closure FACTORY** (the common case): `(def (both (: a Int64)(: b Int64)) (fn ((: x Int64)) (+ (+ a b) x)))`
   — the def's RESULT is `Ty::Fn`. Host: `make(10,20)` → a handle capturing a,b; `call(handle, 5)` = 35.
2. **Closure PARAM**: an export taking a closure argument (rarer; the corpus routes a scalar via the
   wasm handle-ABI). Defer past S1.

## Why NOT mirror wasm
wasm's ABI (`backend/wasm/envelope.rs`) is component-model RESOURCE-based: `resource.new`/`resource.rep`,
a dtor core-module, `make`/`t-encode`/`call` exports off RUN_INTERFACE, funcref dispatch by resource rep.
The rust backend emits native Rust compiled by rustc — no component model. The native equivalent of a
closure handle is simply a Rust closure value: `Box<dyn Fn(Arg) -> Ret>` (or `impl Fn` at the fn boundary).

## The harness gap
`run_program_rust` (xtask:1305) FLATTENS every `Call` arg into one `export(args…)` call. For a factory
export it must instead SPLIT: the first K args are the factory's params (captures), the rest are the
returned closure's args → emit `export(cap0, cap1)(arg0)`. K = the factory's own param count (recoverable
from the emitted `pub fn <name>(` signature's param list, or a descriptor note). The gate `Call` for
`both` is `(both (:10)(:20)(:5))` → K=2 factory params → `both(10, 20)(5)`.

## S1 — base scalar-capture factory (the smallest slice)
Target case: `21-host-closures.sexp:214` "a closure capturing two values is made and called" (both→35).
Also: "a capturing 32-bit-integer closure", "scale"/"let-in-body"/"captured-boolean" variants — all
scalar-capture, scalar-arg factories.

### Backend (`mod.rs` + `expr.rs`)
- In `emit_signature`, when `public && is_fn_ty(result)` AND the result Fn is fully-typed (concrete arg +
  ret): DON'T decline. Instead emit the factory as
  `pub fn both(a: i64, b: i64) -> impl Fn(i64) -> i64 { move |x| { (a + b) + x } }`
  — the captured params (a,b) stay outer-fn params; the returned `(fn (x) …)` body becomes a `move`
  closure. `impl Fn` (not `Box<dyn Fn>`) keeps it zero-cost + avoids a boxing decision; if a shape needs a
  named/boxed type (multi-export sharing one signature — S4), promote to `Box<dyn Fn>` then.
- The closure body is ALREADY what `expr::emit` produces for a `Core` lambda internally — reuse the
  existing internal-closure lowering, just place it as the fn's return expr with `move`.
- Guard: only lift when the Fn's arg + ret types map natively (else keep declining). Async mode: defer
  (thread env) — S1 is sync-only.

### Harness (`run_program_rust`, xtask)
- Detect a factory export: the emitted signature is `pub fn <name>(<params>) -> impl Fn(<...>) -> <...>`
  (grep the return arrow for `impl Fn`/`Box<dyn Fn`). Recover K = the factory param count.
- Split `Call.args`: first K → factory call, rest → applied to the returned closure. Emit
  `<name>(cap_args)(call_args)`. Render each arg via the existing `rust_call_arg`.
- The result renders through the same `cdz_render_at` path (the closure's Ret type).

### Gate + tests
- Unit: `compile_rust` a factory → assert `-> impl Fn(` + `move |`; `rustc_run` split-call → value.
- The corpus `21-host-closures.sexp` scalar-capture cases flip todo→pass on rust (baseline refresh,
  additive; verify --check).

## S2 — compound ARG (Tuple/Option/Result/List closure arg)  → after S1
The returned closure takes a compound arg: `impl Fn((i64,i64)) -> i64`. Harness rebuilds the compound arg
(the existing `rust_call_arg` already does tuple/record/Option/Result/list). Mostly harness-side once S1's
split exists.

## S3 — compound RESULT (List/tuple/String/Bytes/sum closure return)  → after S2
The closure returns a compound; render via `cdz_render_at`. Coordinate with v-cdz-tooling's cdz-rust-render
extraction (the render surface grows here).

## S4 — MULTI-EXPORT / DISTINCT-SIG  → last
N factories sharing shapes; may need `Box<dyn Fn>` named types instead of `impl Fn`. + capturing closures
of different signatures. Also the closure-PARAM export shape (shape 2 above).

## SHAPE 2 — closure-PARAMETER export (the CONSUMER) — ✅ SYNC SHAPE LANDED (2026-07-21, trunk `eaedfcf85`)
**STATUS:** the SIMPLE sync shape (scalar arrow spine + a producing sibling) LANDED as MR `b697a1984`
(re-land after the multi-reject saga). Emit guard-lift in `mod.rs` (`closure_param_is_simple` +
`has_producer_for`) + harness producer→consumer wiring (`build_closure_consumer_call` in xtask, arrow-aware
`parse_emitted_sig`). Async closure-param consumers, HIGHER-ORDER params, COMPOUND closure arg/result, and
Bytes/String export results still DECLINE (clean todo). ⏭️ NEXT increment = **S2 for the consumer**: widen
`closure_param_is_simple`'s arg check to reuse the factory-side `s2_arg_ok` (Tuple/List/Option/Result over
OK elements) so a compound-closure-arg consumer emits; the harness `build_closure_consumer_call` already
rebuilds compound producer captures via `rust_call_arg`, so verify it drives a compound closure arg before
lifting. ⚠ MERGE-FRAGILE (resaves both rust + rust-async baselines — the async-saga profile): single commit,
minimal title-aligned baseline diff, land when the host is calm + I'm the sole baseline-toucher in the batch.

**~85 cases** — was the LARGEST remaining closure family (measured across 21-host-closures + 09-functions on
both rust + rust-async). Decline site (BEFORE the land): `mod.rs:580` — `if public &&
params.iter().any(is_fn_ty) → "a closure PARAMETER cannot cross the Rust export boundary"`. BOTH-backend
(fires on sync rust AND rust-async), so the one slice improved both.

### The shape
A CONSUMER export takes a closure param + applies it: `(def (twice-plus (: g (-> Int64 Int64)) (: x Int64))
(+ (g x) (g x)))`. The body already lowers fine (`g x` → `Core::CallClosure` → `(g)(x)` emit). Two pieces
are missing:
1. **Emit side — ✅ PROVEN TRIVIAL (detached-worktree probe, 2026-07-20): just LIFT the `mod.rs:580` guard,
   nothing else.** Probed by lifting the guard in a throwaway `git worktree --detach` at trunk (never
   touched my queued branch) and compiling `twice-plus`: it emitted `pub fn twice_plus(g: std::rc::Rc<dyn
   Fn(i64) -> i64>, x: i64) -> i64` with body `((g.clone())(x)).checked_add((g.clone())(x))…` and **rustc
   compiled it clean, zero other changes**. `rust_type(Ty::Fn)` already yields `Rc<dyn Fn>`, and
   `CallClosure` already emits `g.clone()` per application (so the applied-twice `Fn`-not-`FnOnce` worry is
   a NON-issue — the `.clone()` is already there). The lowering (`lower.rs:1045`, `Resolved::Param` head →
   `Core::CallClosure`) + the wasm backend already accept this program, confirming it's rust-emit-only. So
   the emit slice ≈ delete the guard block + confirm no regression (a bare-`fn`-value RETURN vs a
   fn-PARAM: the guard currently also has to not re-block the S1/S2 factory path above it — check ordering).
2. **Gate harness (the real work):** the corpus `(call twice-plus (: 1 …) (: 5 …))` supplies `1` as the
   CAPTURE for a COMPANION PRODUCER (`make-adder`) and `5` as the consumer's scalar arg — mirroring the wasm
   make/call handle ABI: the host builds the closure via `make-adder(1)` → an `Rc<dyn Fn>`, THEN calls
   `twice-plus(that_rc, 5)`. So `run_program_rust` must, for a consumer export whose param is `Ty::Fn`:
   locate the companion producer export, build its `Rc<dyn Fn>` from the leading call args, and pass it as
   the closure param. This is the analogue of the wasm gate's producer→handle→consumer wiring. Non-trivial
   harness logic (identify producer, split args producer-caps | consumer-args, thread the Rc).

### Sub-shapes (in the corpus)
- consumer applies the closure ONCE / TWICE / inside a larger expr (`(+ (g x) (g x))`).
- closure param NOT first (component functype follows SOURCE order).
- MULTIPLE closure params of the same signature (`app2(h1,h2,5)` — fresh handle per param).
- wider scalar widths (Int32/UInt64/Int8/Float32/mixed `(-> Int32 Bool)`) — each crosses; the rust param
  type follows `rust_type` per width, already supported.
- compound closure ARG / RESULT — composes with S2/S3 render once the base consumer shape lands.

### HARNESS MECHANISM (scoped 2026-07-20 — the net-new work)
🔑 On WASM the gate harness is TRIVIAL for a closure-param consumer: `run_program_wasm` just passes
`--call twice-plus --arg 1 --arg 5` to **cdz-run**, and CDZ-RUN ITSELF synthesizes the closure argument
(it knows `twice-plus`'s first param is a closure RESOURCE and builds it from the companion producer via
the component-model resource/handle ABI). So wasm "just works" — the harness does nothing special.
On RUST there is NO cdz-run equivalent: `run_program_rust` compiles the emitted rust + a hand-written
`fn main` driver, so the driver must ITSELF synthesize the `Rc<dyn Fn>` argument. This is the net-new work.
The corpus `(call twice-plus 1 5)` for a program with exports `make-adder` (producer) + `twice-plus`
(consumer): arg `1` builds the closure via `make-adder(1)` → an `Rc<dyn Fn>`, then `twice-plus(that_rc, 5)`.
Driver must: (1) detect the consumer export's `Ty::Fn` param (parse the emitted `pub fn <name>(… : Rc<dyn
Fn(…)>, …)` sig — a param whose rust type starts `std::rc::Rc<dyn Fn`); (2) find the companion PRODUCER
export whose `Rc<dyn Fn>` RESULT type matches that param type; (3) split the flat call args: the producer's
own params (leading) build the closure, the rest drive the consumer; (4) emit `let __g = prog::make_adder(
<caps>); prog::twice_plus(__g, <rest>)`. The producer-match is by rust-type-equality of `Rc<dyn Fn(A)->R>`
(the FACTORY `rust_factory_param_count` machinery already parses factory sigs — reuse it to identify the
producer). MULTI closure-param (`app2(h1,h2,5)`): each Fn param gets a fresh producer-built closure.

### Sequencing
Do this slice FIRST among the remaining closure work (biggest payoff, both-backend). The EMIT half is a
~1-line guard-lift (PROVEN in a detached-worktree probe: lifting `mod.rs:582` emits correct `pub fn
twice_plus(g: Rc<dyn Fn(i64)->i64>, x)` that rustc-compiles). The HARNESS producer→consumer wiring (above)
is the bulk. ⚠ MERGE-FRAGILITY: this flips ~85 cases → a large baseline resave on BOTH rust + rust-async,
the exact profile that caused the 5-reject async-closure saga. MITIGATION: land as a SINGLE commit, minimal
baseline diff (verdict flips only, title-aligned), full `cargo test --workspace`, parented on current trunk,
sent when I'm the sole baseline-toucher in the batch if possible. Gate: 21-host-closures consumer cases flip
todo→pass on BOTH rust + rust-async. Then S2/S3 compound arg/result, then the tiny option-A (3 cases).

## Coordination
- v-cdz-tooling: the cdz-rust-render extraction — closure Ret render cases land in S3; ping them on the seam.
- Async (rust-async target): the closure must thread the gas/yield env; a separate sub-slice per S-level.
  (For a closure PARAM the env threading is on the CONSUMER's own `async fn`, not the passed closure — the
  passed `Rc<dyn Fn>` stays sync per the option-C rule, so no boxed-future ABI needed for shape 2.)
