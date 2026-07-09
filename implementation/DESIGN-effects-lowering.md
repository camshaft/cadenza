# Design — lowering intra-program effects (`handle` / perform / `resume`) to wasm

**Author:** compiler engineer. **Audience:** whoever grows `cdz-compiler/src/codegen.rs` next.
**Status:** **STAGES 0–3 LANDED (2026-07-06).** The spec-side decision is pinned in
`options/effects-model/lowering-to-wasm.md` (the *what*); this is the *how* against the real seed, with
exact structs, hook sites, and emitted-byte shapes.

> **LANDED — what's realized in the seed now (gate: 469 pass, 0 fail):**
> - **Stage 0** — routing-agnostic `(effect …)` parsed (`collect_effects` → `effects` table on
>   `Compiler`); `effect`/`op`/`handle`/`host`/`resume` are form keywords; `gen_perform` in the
>   `.`-headed dispatcher; `gen_handle`/`gen_host` in the keyword match; router stack
>   (`RouterFrame::{Handler,Host}`) on `FnCtx`; classifier (`classify_arm` → Tail/Abortive/GeneralOneShot);
>   `CDZ0401`(merged)/`CDZ0403`/`CDZ0404`; `parse_host_imports`/`declared_capabilities` **deleted**; the
>   manifest is **computed** (`compute_manifest`) from the entrypoint's `(host …)` delegations, and
>   `(meta capabilities)` reads from it (`collect_host_delegations`).
> - **Stage 1** — Tier-1 tail inline (`emit_handler_arm` + `unwrap_tail_resume`); the under-frame
>   (`ctx.routers.split_off(def_depth)`); **state threading** via a mutable wasm local
>   (`emit_tail_resume_threaded`, `@state-local` accessor node), unit-state = zero-cost degenerate;
>   cross-function resolution by **inlining** effectful callees (`fn_reaches_effect` gate in `gen_call`),
>   guarded against recursion by `ctx.inlining` — a **recursive effectful function DECLINES cleanly**
>   (never hangs), the Stage-3 boundary.
> - **Stage 2** — host delegation → boundary call: **effect = WIT interface, op = function in it**
>   (`host_import_component` rewritten to import one instance per effect — dissolves the invalid dotted
>   extern-name `log.emit`), `Unit` params/results stripped at the boundary; `bind_host_imports` (host.rs)
>   binds interface-nested funcs and records the call as `effect.op`; interpose-and-forward works via the
>   under-frame. `shape_of`/`infer_list` see through `handle`/`host`/perform so a compound-returning
>   handler drives the runtime-compound path.
>
> - **Stage 3** — effect-context monomorphization for *recursive* effectful functions, realized by
>   **args + multi-value return** (NOT a global — a global holds one live state per effect and clobbers
>   on nesting/re-entry). A recursive effectful function is emitted once per handler context as a real
>   wasm function `f#ctx(orig-params…, s_in…) -> (result, s_out…)` (`gen_specialized_call`,
>   `intern_specialization`, `emit_specialization_body`, `functype_spec`); each enclosing handler's
>   state is a hidden trailing param + extra return, so nested/wrapped effects compose (witnessed:
>   two-handler-state recursion → 30, mutual recursion → 104, countdown → 3, range-sum → 6). The
>   self-call resolves to the reserved specialization slot as a plain `call`. Only wired on the
>   **self-contained scalar** path; a spec on the runtime-compound/host path declines. A recursive
>   function that installs a FRESH handler per call grows the context without bound → capped
>   (`MAX_SPECS_PER_FN`) and declines (previously overflowed the compiler stack — a corpus case now
>   guards it).
>
> **NOT landed (clean declines, honest todos):** Tier 2 abortive; Tier 3 general one-shot / captured
> continuation; a recursive effectful function returning a compound or under a host delegation.

This is a how-to, not a mandate. The strategy (classification-first; tail-resumptive is plain code) is
settled and validated; the code shapes below are the concrete realization I'd reach for, matched to the
seed as it stands at this commit. Where I name a line number it's a landmark, not a promise it won't drift.

---

## 1. TL;DR — the win, and the one insight

**The win.** Effects are the #1 self-hosting blocker (the compiler-in-Cadenza spike counted 10 `DECLINE`
markers for effects, ahead of everything else). The compiler's own ambient state — a fresh-name supply
(`Fresh`), diagnostics (`Diag`), a unification store (`Unify`) — is expressed as intra-program effects.
Lowering them unblocks authoring the compiler in Cadenza.

**The insight that makes it cheap.** Cadenza resolves the discharging handler **statically, at compile
time** — the handler is the nearest one enclosing the perform in *dynamic extent* (the call chain), but it
is determined statically by monomorphizing the handler context over a **closed effect row** (§7.5), so at
every perform site the discharging handler arm is a *compile-time constant*. And every handler arm the
corpus and the compiler use is **tail-resumptive** (`op ↦ resume value next-state`, resume in tail
position). A tail-resumptive perform against a statically-known arm is **an ordinary inlined function
body**: bind the operation's parameters to the arguments, and emit the arm body with `(resume value
next-state)` replaced by `value` (threading `next-state`). **No continuation object, no handler stack at
runtime, no evidence vector, no new runtime op, no envelope re-derivation.** It reuses the exact machinery
the seed already has for inlining a lambda argument (`Local::aliased` + `emit`).

Everything here is stock wasm. The general (non-tail) one-shot case — which no current program reaches —
is quarantined behind a precise trigger and, until built, is a clean decline.

---

## 2. What's already true in the seed (so we build on it, not around it)

- **`Local` is either a runtime slot or a compile-time alias.** `Local::aliased(name, node, env)` binds a
  name to *an AST node re-emitted under a captured environment* — no runtime storage
  (`codegen.rs` ~5467). This is how a lambda parameter, a `let` of a structural value, and a HOF's lambda
  argument all already flow. **Tier-1 perform is the same move.**
- **The `has_lambda_arg` path inlines a call by aliasing params to arg nodes** and emitting the callee
  body (`codegen.rs` ~5028): `for (p,a) in params.zip(args) { body_env.push(Local::aliased(p, a, env)) }
  ; self.emit(&f.body, &body_env, ctx)`. A tail-resumptive arm is emitted *identically*, with one extra
  rewrite (`resume` → its argument).
- **A `.`-headed list is already a dispatch point.** `(E.op args)` reads as a list whose head is
  `(. E op)`; the dispatcher already branches on `name_of(hd.first()) == Some(".")` (~2628) and today
  routes to a qualified constructor, lambda projection, or `gen_dotted_apply`. **A perform is a new arm
  there**, checked before those.
- **Host imports are plain imported-function calls.** The guest calls an import and gets its response back
  inline; the entry is a plain function `input -> output` (component-abi.md v4 §The Entry Is A Plain
  Function — the v2 `Suspended` result arm is RETRACTED). How the host produces a response — answer inline,
  suspend a wasmtime fiber and resume in place, or tear down and re-derive from recorded responses — is
  host policy the emitted bytes don't encode; the language guarantees only determinism. Today's seed host
  can trap when it has no response (`cadenza-seed/src/host.rs` ~231 returns `Err`), which is one host's
  choice, not an ABI arm. **Intra-program effects don't touch any of this** — they never reach the boundary.
- **`FnCtx` is per-function mutable state** (`next_local`, `extra_locals`, `called`; ~5550). The handler
  stack lives here.
- **The seed still parses only `(import (host …))`** (`parse_host_imports`, ~8117) and
  `(use (capability …))` (`declared_capabilities`, ~8158). The corpus has migrated to a
  **routing-agnostic** `(effect Name (op …))` declaration plus an **entrypoint** `(host (Effect…) body)`
  delegation and `Effect.op` performs, so **even the two host-replay cases do not parse today.** Stage 0
  fixes this; it is a prerequisite, not optional. Note both legacy forms are **retired**, not extended:
  host-binding is no longer read from a declaration or a `use`, but from an entrypoint delegation (§9).

---

## 3. The classifier (one pass, shared with CDZ0401/0403/0404)

Add a pass that, for each `(handle <init> (arms…) body)`, classifies each arm `(E.op (p₁…pₙ) s arm-body)`
(the state binder `s` follows the op params) by how `arm-body` uses `resume` (the two-argument
`(resume value next-state)`). Treat `resume` as bound only within this arm — do **not** descend into a nested
`(fn …)` or nested `(handle …)` when counting.

```
enum ArmClass {
    Tail,                     // resume once, tail position — the whole corpus. State threads through resume.
    Abortive,                 // resume never called (exception shape)
    GeneralOneShot,           // resume not in tail, or continuation captured — needs reification
}
```

**State is not a class — it is a slot every arm carries.** The surface is uniform (decided in this
conversation, 2026-07-06): every `(handle <init> (arms) body)` seeds an accumulator, every arm binds the
current state, and `(resume <value> <next-state>)` is always two-argument. So there is no separate
`TailPure`/`TailState` distinction at the class level — a tail arm *always* threads state; the "stateless"
handler is the degenerate `<init> = unit`, `<next-state> = s` case, which costs nothing because
`Kind::Unit` emits no bytes (§5). The state-passing transform is the *one* Tier-1 lowering; the unit-state
fast path (skip the transform when the state kind is Unit) makes today's cases byte-identical. This
collapses the former Tier-1/Tier-1′ split into a single tail lowering with a zero-cost degenerate case.

Decision procedure over `arm-body` (syntactic, exact):

1. `resume_count` = number of `(resume _ _)` occurrences not under a nested `fn`/`handle`.
2. If `resume_count == 0` → **Abortive**.
3. If `resume_count == 1` **and** the single `resume` is in **tail position** of every control path (the
   arm's tail; each tail branch of an `if`/`match` that is itself in tail position; the last form of a
   tail `do`) **and** the resumed continuation is not otherwise named → **Tail**. Tail position is the
   recursive definition the seed already uses to decide where a value is produced; reuse it.
4. Otherwise → **GeneralOneShot**.

**Conservatism is a correctness requirement.** Mis-classifying a non-tail arm as tail silently drops the
post-`resume` work — a miscompile, the one thing this project never does. Anything not *provably* Tail or
Abortive is GeneralOneShot, and GeneralOneShot declines until Stage 5.

**Refinement — tier is a static join over ALL control paths; a runtime branch never changes tier.** The
reify-or-not decision is made at the **perform site, upstream of the arm body**: a tail perform inlines
the arm and uses its return; a capturing/non-tail perform builds the frame *before* calling the arm. The
perform site cannot see which branch a runtime `if` inside the arm will take, and on stock wasm you cannot
retroactively reify the delimited region after the branch runs (that is exactly why native stack-switching
is rejected). So an arm's class is the **least upper bound over every syntactic control path**: if any path
captures `k` or resumes non-tail, the whole arm is `GeneralOneShot` and the perform reifies
unconditionally; the runtime branch then runs *inside* the already-built frame. A branch chooses behavior,
never tier.

**Refinement — resume-vs-abort is a frame-free hybrid, not `GeneralOneShot`.** An arm whose branches each
either resume-in-tail *or* abort (e.g. `(if c (resume e s) V)`) needs **no reification**: inline it, the
resume branch falls through with `e` (Tail) and the abort branch does `br $handle_end` with `V` (Abortive).
Both control targets are in scope at the perform site. The binary decision procedure above lumps this into
`GeneralOneShot` and over-declines it; recognize the resume-or-abort shape as a distinct frame-free class
when it appears (no corpus case needs it yet, so the over-decline is currently harmless). The precise line:
only a genuinely non-tail *resume* or a *captured* `k` needs a frame; a branch that merely chooses
"resume-tail vs abort" does not.

**Option — make the capture trigger an explicit surface declaration (`fun`/`ctl`).** Today capture is
*inferred* (an arm that stores/returns `k` is reclassified to `GeneralOneShot`, which declines until Stage
5), so adding one line after `resume` can silently flip a program from "compiles" to "declines" — spooky
action at a distance. Koka avoids this by splitting the surface: a `fun` clause is tail-resumptive by
construction (the continuation is *not* in scope — try to capture it and it is a compile error) while a
`ctl` clause binds `k`/`resume` as a first-class value and gets the full control machinery. Cadenza's
proposed continuation binder is the same split: a bare `(handle <init> (arms) body)` arm has no `k` in
scope and *cannot* capture (it can only resume tail or abort), while a `(handle/k <init> (arms) body)` arm
binds `k` and is where capture/non-tail live. Adopting this turns "silently reclassified to a tier that
does not exist yet" into a hard, local error, and gives a stable cost model (bare = frame-free; `/k` =
reified). Note the split governs the **capture** trigger only; the **non-tail-return** trigger
(`(+ 1 (resume e s))`) is still inferred from `resume`'s syntactic position even in a bare handle — unless
you additionally require bare-handle arms to be strictly tail-or-abort, which would make the surface a
complete honest declaration of the tier (bare ⇔ frame-free). Recommended; decide when Tier 3 is built. So the failure mode is "declines",
never "wrong".

**Verified against `spec/semantics/14-effects-and-handlers.sexp`:** every arm is Tail — under the uniform
state surface each resumes once in tail position, e.g. `Choose.pick`(`resume 5 s`), `Get.get`(`resume 41
s`), `Scope.resolve`(`resume x s`) thread state unchanged (unit-state, degenerate), while `Fresh.next`
(`resume s (+ s 1)`) and `Diag.emit`(`resume unit (List.push s code)`) genuinely fold a non-unit
accumulator. None captures `k`; none resumes non-tail. So the fast path is the whole corpus, and the
stateful cases exercise the state-passing transform without leaving the tail class.

**Same pass emits the rejections** (all codes registered in
`options/diagnostics-schema/coded-span-record.md`):
- an arm naming `E.op` where `op ∉ decl(E)` → **CDZ0403**;
- a performed op reached with **neither** an enclosing handler **nor** an enclosing entrypoint `host`
  delegation → **CDZ0401**. This is the single "no home for a reached effect" check — it **merges** the
  former undischarged-intra `CDZ0402` and the former undeclared-host `CDZ0401`, which are one condition now
  that host-binding is an entrypoint routing decision, not a declaration-time property. `CDZ0402` is a
  reserved, no-longer-emitted number;
- an entrypoint `host` delegation naming an effect the delegated body never reaches → **CDZ0404** (latent
  authority — the manifest must be exactly the delegated-and-reached effects).

---

## 4. The handler stack (compile-time evidence, no runtime cost)

On `FnCtx`, add:

```rust
struct HandlerFrame {
    effect: String,                 // "Scale"
    arms: Vec<HandlerArm>,          // one per named op
    init: Node,                     // the seed state expression from (handle <init> …)
    state_kind: Kind,               // Unit ⇒ zero-cost fast path (thread nothing)
}
struct HandlerArm {
    op: String,                     // "by"  (resolved name is "Scale.by")
    params: Vec<String>,           // ["n"]
    state: String,                  // the state binder, e.g. "s"  (bound to current handler state)
    body: Node,                     // arm body, resume UNREWRITTEN (resume value next-state)
    class: ArmClass,
    def_env: Vec<Local>,            // environment captured AT THE HANDLE SITE
    def_depth: usize,               // handler-stack length AT THE HANDLE SITE  ← the under-frame
}
// on FnCtx:
handlers: Vec<HandlerFrame>,
```

`gen_handle(elems, env, ctx)` — form is `(handle <init> (arms…) body)`:
1. Parse the seed `<init>` and arms; classify each; check CDZ0403; record `state_kind` (from `<init>`).
2. `ctx.handlers.push(frame_with_init_def_env_and_depth)`.
3. `let (body_c, body_k) = self.emit(&body, env, ctx)?;`
4. `ctx.handlers.pop();`
5. Return `(body_c, body_k)`. **The handle form emits only its body** — the arms are emitted lazily at
   each perform site (Tier 1 inline) or once as functions (Tier 1b).

The router stack holds **two** frame kinds — `HandlerFrame` (a lexical `handle`) and `HostFrame` (an
entrypoint `host` delegation, naming a set of effects and pushed by `gen_host`; §9). Both resolve by the
same top-down nearest-enclosing rule, so a `handle` nearer a perform than an enclosing `host` **interposes**
on an otherwise-delegated effect — this is how mocking/counting falls out for free.

`gen_perform(effect, op, args, env, ctx)` — reached from the `.`-headed dispatcher arm when `effect` is a
declared effect and `op ∈ decl(effect)`:
1. Resolve top-down over the router stack. The nearest match is either:
   - a `HandlerFrame` for `effect` with an arm for `op` → dispatch on `arm.class` (step 2); or
   - a `HostFrame` that delegates `effect` → emit the host-boundary call (§8), and mark the effect reached
     so the entrypoint's CDZ0404 latent-authority check passes.
   None found by the top of the entrypoint → **CDZ0401** (no handler, no delegation).
2. Dispatch on `arm.class` (HandlerFrame case):
   - **Tail** → §5 inline (with state threading; unit-state is the zero-cost degenerate case).
   - **Abortive** → §6.
   - **GeneralOneShot** → §7 (decline until Stage 5).

---

## 5. Tier 1 — tail-resumptive → inline the arm, unwrap `resume`, thread state

This is the whole shipping surface. For a Tail arm `(p₁…pₙ) s arm-body`:

```rust
// bind each param to its argument node (and s to the current state), exactly like has_lambda_arg
let mut body_env = arm.def_env.clone();
for (p, a) in arm.params.iter().zip(args) {
    body_env.push(Local::aliased(p.clone(), a.clone(), env.to_vec()));
}
body_env.push(Local::aliased(arm.state.clone(), cur_state.clone(), env.to_vec())); // unit ⇒ zero-width
// emit the arm body with resume unwrapped, under the handler stack truncated to def_depth
let saved = ctx.handlers.split_off(arm.def_depth);   // ← the under-frame (see landmine)
let rewritten = unwrap_tail_resume(&arm.body);        // (resume value next-state) ⇒ value; thread next-state
let out = self.emit(&rewritten, &body_env, ctx);
ctx.handlers.extend(saved);                           // restore
out
```

`unwrap_tail_resume(node)`: structurally replace the tail `(resume value next-state)` with `value`
(recursing into the tail of `if`/`match`/`do` the same way tail position was computed), and thread
`next-state` forward as the handler state seen by the rest of the handled region (§the state transform
below). `resume` is **not** a call and has no runtime representation — unwrapping it *is* the lowering; the
second argument is where the accumulator flows, not a control operation.

**Worked shapes (byte-level intuition):**
- `(handle unit ((Get.get () s (resume 41 s))) (+ (Get.get) 1))` → state is Unit (zero-width, threaded
  trivially); `(Get.get)` emits `41` → `(+ 41 1)` const-folds → `i64.const 42`. Byte-identical to a
  stateless handler because `Kind::Unit` emits no bytes.
- `(handle 0 ((Fresh.next (u) s (resume s (+ s 1)))) (do (Fresh.next) (Fresh.next) (Fresh.next)))` →
  a genuine `Int64` accumulator seeded `0`; each `(Fresh.next)` yields the current `s` and threads `s+1`
  forward, so the three performs see `0,1,2` and the `do` yields `i64.const 2`. This is the state-passing
  transform doing real work.
- `(handle (list) ((Diag.emit (code) s (resume unit (List.push s code))) (Diag.collect (u) s (resume s s))) (do (Diag.emit 201) (Diag.emit 210) (Diag.collect)))`
  → the accumulator is a list handle seeded empty; each `emit` threads `List.push s code` forward; the
  final `collect` reads the accumulated `(list 201 210)` out *as an ordinary operation* (its arm resumes
  `s` and threads `s` unchanged) — no separate return clause, the read-out is just an op.
- The composition case `(handle unit ((Scale.by (n) s (resume (* n 2) s))) (Scale.by (ask.ask)))`:
  `(ask.ask)` is a host call (emits the host-call sequence, result on the stack as an i64), bound to `n`,
  arm emits `(* n 2)` (state Unit, threaded trivially). If the host log has `21`, run yields `42`; if not,
  the host call suspends and the whole stack unwinds — **the intra-program handler left nothing to
  preserve.**

**The under-frame landmine (verified "surprisingly subtle").** If an arm body *itself* performs an
operation, that nested perform must resolve against the handlers in scope **where the arm was defined**
(`def_depth`), not where the operation was performed. Truncating `ctx.handlers` to `arm.def_depth` before
emitting the body is what makes this correct. Skip it and nested same-effect handlers silently
miscompile (`h2 = { ask ↦ resume (perform ask + 1) }` under an outer `ask` handler resolves to the wrong
arm). Test a two-deep same-effect handler before declaring Stage 1 done. This is *why* arms carry
`def_env`/`def_depth` rather than being naively substituted, even though resolution is static.

**Kind.** No new `Kind`. The perform's result kind = the operation's declared result type through the
existing type→Kind path; it's just the kind `emit` returns for the rewritten body. The handle expression's
kind is its body's kind.

### Tier 1b — emit-as-function for reused/large arms (correctness-neutral)

Inlining at every perform site blows up code size if an op is performed at many sites (and the seed is
already exponential in deep nesting — a standing scale bug). When an arm is Tail, large, and performed
more than once, emit it **once** as an ordinary wasm function taking the params (and the state), with
`resume value next-state` → the function's return of `(value, next-state)`, and emit each perform as
`call`. Reuses the existing `op::CALL` + `call_base` + `ctx.called.insert(idx)` reachability path (~5061).
Pure optimization; identical observable result.

### The state transform (folded into Tier 1, not a separate tier)

Every `handle` threads state, so the state-passing transform IS the Tier-1 lowering — not a distinct
sub-tier. Over the handled region: the resume value/next-state pair threads left-to-right through the
continuation, so each performed operation reads the current state (bound to the arm's `<state>` binder) and
`(resume value next-state)` delivers `value` to the perform site while carrying `next-state` forward to the
rest of the region. A read-out operation (`Diag.collect`, or a general `State.get`) is just an arm that
resumes the current state as its value; a write (`State.set`, or `Fresh.next`'s advance) is an arm that
threads a new state. The state is a scalar or an opaque `Kind::Heap` handle threaded by value — the heap
stays immutable (`capabilities-and-effects.md` §A Handler Threads State Across The Operations It
Discharges). The handle evaluates to the **body's** value; the accumulated state is discharged at the
handle boundary (read out only if the body performed a read-out op).

**The unit-state fast path is mandatory, and it is what makes this free.** When the handler's state kind is
`Kind::Unit` (seed `unit`, arms thread `s` unchanged), the transform threads a zero-width value —
`Kind::Unit` emits no bytes — so the emitted code is byte-identical to a stateless inline. This is why
collapsing the old TailPure/TailState split costs nothing: the "pure" case is literally the Unit instance
of the one transform. Detect it by the seed's existing `Kind::Unit` check and skip threading; do **not**
emit a dummy local. The corpus's five unit-state handlers (`Choose`/`Get`/`Scale`/`Count`/`Unify`/`Scope`)
must produce the same bytes they would have without state; the two non-unit handlers (`Fresh` counter,
`Diag` list) exercise the real transform.

**Reused/large stateful arms** ride Tier-1b unchanged — the emitted function returns `(value, next-state)`
instead of just `value`; a Unit next-state collapses that back to a bare `value` return.

---

## 6. Tier 2 — abortive → `block` + `br`

An arm that never resumes is an early exit. Emit the handled `body` inside a wasm `block` whose result
type is the handle's result Kind; lower each perform of an abortive op to `br` to that block's end,
leaving the arm's value on the stack. No capture, no continuation. (Could instead target the finished
wasm exception-handling proposal, but `block`/`br` needs nothing extra and is preferred.) No corpus case;
build opportunistically when an exception-shaped effect appears.

---

## 7. Tier 3 — general one-shot → defunctionalized frame on the value heap (the fallback)

**Trigger:** `ArmClass::GeneralOneShot`. No corpus case and no compiler effect reaches this. **Until
built, `gen_perform` declines** — an honest backlog entry, never a miscompile.

When built: reify the delimited region between the perform and its statically-known handler as a
first-order frame on the **existing** value-heap runtime (defunctionalization):

- A frame = `sum-new(site_disc, arr_of_captured_locals)` — `site_disc` a compiler-assigned discriminant
  per suspension point; the payload array (`arr-alloc`/`arr-set`) holds the live locals there. All in the
  **frozen WIT prefix** (sum 10–12, arr 6–9). **No new runtime op; no envelope re-derivation.**
- Non-tail perform: build the frame, call the statically-known handler arm passing `(args, k)`, `k` = the
  frame handle.
- `resume k v` = call a compiler-emitted `apply` function: read `sum-disc(k)`, `br_table` to that site's
  code, restore locals from `sum-payload(k)` via `arr-get`, resume yielding `v`.
- One-shot ⇒ the frame is consumed once; the runtime's existing RC `drop` reclaims it. Multi-shot (rare,
  build-level opt-in) copies the frame chain per resume — a cost, not a soundness break; statically reject
  a second `resume` unless the build enables it.

If a dedicated frame rep is ever wanted it's an **append-only** WIT op at a new frozen index (like
`bytes-compact` at 36) — one envelope re-derivation, never a reshuffle. But the `sum`+`arr` encoding
needs none.

---

## 7.5 Effect-context monomorphization — the bridge across function boundaries

Everything above describes resolution **within one function** — `ctx.handlers` is per-`FnCtx`, so a
perform resolves against handlers pushed by `gen_handle` calls in the *same* function body. But handler
resolution is **dynamic in extent** (`capabilities-and-effects.md` §Handler Resolution Is Dynamic In Extent
And Statically Determined): the perform can be in a *callee* while the `handle` is in a *caller*. The corpus
cross-function cases pin this — `gen` performs `Bump.by`, `main` handles it (`→ 42`); `ask` performs
`Get.get` and is called under two different handlers (`→ 32`); a perform resolves through three transparent
intermediate frames (`→ 10` deep-chain). None of these resolve within the performing function's own
`ctx.handlers`. So the intra-function stack is **necessary but not sufficient**; this section is the missing
piece between it and Tier 3.

**The model: an effect is an implicit evidence parameter.** A function that (transitively) performs an
operation it does not itself handle takes the discharging handler as an *implicit parameter*, threaded from
whichever caller installed it — exactly Koka's evidence passing / Effekt's capabilities. Resolution is
dynamic (which handler is decided by the call chain), but **statically determined** because we specialize:

> **Monomorphize each effectful function once per handler-context it is called under.** In each specialized
> copy, the discharging arm for every perform is a compile-time constant — the intra-function `ctx.handlers`
> machinery above then applies verbatim, because the caller's handler has been made lexically present in the
> specialized body.

This is the compile-time collapse of the runtime evidence vector: the "evidence" is not passed at runtime,
it is a specialization key. `same-fn-two-handlers → 32` is the witness that one *definition* becomes two
*specializations* (`ask` under the `resume 10` handler, `ask` under the `resume 20` handler), each with its
`Get.get` resolved to a different constant arm.

**Two ways to make the caller's handler lexically present** (they compose):

1. **Inlining** — what the seed does *today* for lambdas/HOFs via `Local::aliased` + `emit` (§2). Inline
   the callee into the handled region and its performs become textually enclosed; intra-function resolution
   just works. This is the natural realization of the corpus cross-function cases at Stage 1: they are
   non-recursive, so `gen`/`ask`/`mid`/`leaf`/`a..d` inline into the handle body and the existing machinery
   resolves them. **Stage 1 gets cross-function-via-inlining for free** — no new mechanism, and it turns the
   cross-function corpus cases green alongside the same-function ones.
2. **Effect-context specialization** — emit the function once as a real wasm function *per distinct handler
   context*, with the context recorded as a specialization parameter (a monomorphization key alongside the
   type-monomorphization the seed already does for generics). Needed when inlining doesn't terminate or
   duplicates too much — see below.

**Where pure monomorphization breaks (the load-bearing limit).** Specialization is total only when the
handler-context of every call is statically known. It fails for **the same function *value* invoked under
different handlers chosen at runtime** — e.g. a closure that captures a handler context at creation and is
later called under another, or a task thunk that could be spawned under handler A or B. There is no single
copy to pick. This is exactly:

- **the recursive-task wall a scheduler hits** — a process loops (`sleep`; loop), so it cannot be inlined
  into the handle (non-terminating), and it is resumed by the scheduler under a context chosen at runtime.
  That forces the function to be emitted in **resumable form** — its perform sites get `site_disc`s and its
  live locals get captured into frames — and the continuation spanning `task → … → handle` becomes the
  **Tier-3 linked frame chain** (§7). *This is why a scheduler needs Tier-3 and not just inlining:*
  monomorphization resolves *which* arm, but a recursive task cannot be made lexically present, so its
  continuation must be reified as data. Tier-3's `apply`/`br_table` is how a reified continuation crosses
  function boundaries without inlining.
- **the Koka-dynamic vs Effekt-lexical-capability fork.** Both are cross-function-capable via the same
  implicit-evidence threading; they diverge on precisely this case (does a captured-then-invoked closure use
  its creation-site or its invocation-site handler?). So "dynamic vs lexical-capability" and "when does
  monomorphization need a runtime-evidence fallback" are the **same question**. When the static copy can't
  be chosen, the fallback is to pass evidence at runtime (a handler handle threaded as a real argument) or
  defunctionalize — deferred with Tier 3.

**Precise contamination rule (not "the whole handler goes Tier-3").** A runtime branch never changes an
arm's tier (§3), and likewise a `Tail` perform inside a function that *also* contains a reachable
`GeneralOneShot` perform still inlines for free — it does not get a frame. The rule is: *a function
containing a reachable non-tail/captured perform must be emitted in resumable form; but the tail performs
within that resumable body still lower to plain inlined code.* Only the reifying sites cost a frame.

**Seed impact.** Stage 1 does cross-function by inlining (no new mechanism; turns the 6 cross-function
corpus cases green). Effect-context specialization as a distinct emit path (a monomorphization key on
`FnCtx` emission, sharing the generics-monomorphization machinery) is only needed when inlining is
non-terminating or over-duplicates — i.e. the moment a recursive effectful function or a scheduler appears,
which is also the moment Tier 3 is needed. Until then, inlining subsumes it. Watch the seed's known
exponential-in-nesting blowup: inlining effectful callees adds IR, so prefer Tier-1b emit-as-function for
callees performed at many sites, and specialization (not inlining) for large recursive ones.

---

## 8. Composition with the host boundary — the invariant

A host-delegated effect performed *inside* an intra-program handler (corpus: "a delegated effect performed
inside an intra-program handler suspends and replays", and "an intra-program handler interposes on a
delegated effect, counts it, and forwards to the boundary"). Because a Tier-1/1′/2 handler leaves
**nothing reified on the wasm stack**, the host call is an ordinary imported-function call that returns its
response inline and the run continues — the entry is a plain function, no `Suspended` arm. The compiler
emits the same bytes regardless of how the host resolves the call; the interpose-and-forward case is the
same shape: the forwarding arm re-performs, which resolves (via the under-frame) past itself to the
enclosing `HostFrame` and becomes the boundary call.

The one thing the compiler must guarantee is that a host strategy which **re-derives** the run (drops the
instance, re-invokes with the same input feeding recorded responses in order) reproduces identical
behavior. It does, for free: **deterministic re-execution re-establishes every dynamic handler context and
the entrypoint delegation by recomputation** (which handler discharges each perform is statically fixed, so
re-running the same input reconstructs the same call chain), and nothing intra-program is serialized.
This is why the invariant below matters — a reified (Tier-3) continuation is non-durable heap handles that
a re-derivation could not reconstruct if it were suspended across a host call.

**Compile-time invariant** (checked in the classifier pass, alongside CDZ0401/0403):

> A host-delegated operation may be performed under intra-program handlers only when every enclosing
> intra-program handler up to the delegation is Tail/Abortive. A **reified (Tier-3) continuation
> must not span a host call** (a host may re-derive across it, and a chain of non-durable heap handles is
> not reconstructible from `(input, responses)`).

Today's corpus satisfies it automatically (all tail-resumptive). Enforce it the moment Tier 3 lands.

---

## 9. Stage 0 — the declaration + routing surface (prerequisite, no lowering)

Parsing + rejections, no runtime behavior. This alone turns the rejection cases + the empty-row case green
and stops effect constructs misfiring the dispatcher. **The surface is: routing-agnostic declaration +
entrypoint delegation.** Retire the legacy forms; do not extend them.

- **Retire `parse_host_imports` and `declared_capabilities`.** Host-binding is no longer read from a
  declaration or a `(use (capability …))`. Delete these paths (or leave them dead and unreferenced) — the
  manifest is now computed from entrypoint delegations (below), not from declarations.
- **Parse `(effect Name (op op (-> T… R))…)` — routing-agnostic, no `(host)` marker** — into an
  `effect → {op → (params, result)}` table threaded like the sum-type table. **Same table for every
  effect**; there is no host/intra distinction at the declaration. The operation's WIT signature is the
  `(-> T… R)`, used when the effect is *delegated* to emit the import.
- **Parse the entrypoint delegation `(host (Effect₁ Effect₂ …) body)`** → push a `HostFrame` naming those
  effects onto the router stack (§4), emit `body`, pop. On pop, each named effect that no reachable perform
  matched is **CDZ0404** (latent authority). A delegated effect's operations become the host imports (the
  manifest) — the manifest is the **union of the entrypoints' `HostFrame` effect sets**. **WIT mapping:
  each effect → a WIT `interface`, each op → a `func` in that interface, and the emitted `world` imports
  those interfaces** — exactly the shape the value-heap runtime already uses (`interface heap { box-int:
  func … }` in `runtime.wit`; here `interface log { emit: func(s: string); flush: func(); }` and `world
  program { import log; import clock; export heap; }`). Every op is its own component function, and the
  interface namespace dissolves the same-op-name collision structurally (`log.emit` vs `metric.emit` are
  `emit` in two interfaces) — **no `"effect.op"` flat-string key**. **`host` is admitted only at an
  entrypoint** — a `host`
  form anywhere else is rejected (authority enters strictly from the top; a library never delegates).
- **`is_form_keyword`**: add `"effect"`, `"handle"`, `"host"`, `"resume"` (~7975) so a bare `resume`,
  `handle`, or `host` doesn't fall through to the `unbound name` / `CDZ0401` arm (~5005). (Head-position
  only — a local named `host` or a `.host` field still works; §see the naming discussion.)
- **Dispatcher**: in the `.`-headed arm (~2628), before the constructor/lambda/dotted-apply checks, add
  "if `E` is a declared effect and `op ∈ decl(E)` → `gen_perform`".
- **`gen_host`** alongside `gen_handle` in the name-headed dispatch (§4).
- **Classifier + CDZ0401(merged)/CDZ0403/CDZ0404** as in §3.

**Green after Stage 0:** "a handler arm for an operation the effect does not declare is rejected"
(CDZ0403); "an effect operation reached with neither a handler nor a delegation is rejected" (CDZ0401,
merged); "a delegation naming an effect that is never reached is rejected" (CDZ0404); "a program that
delegates no effect is pure and never suspends" (`+ 20 22 → 42`).

---

## 10. Staging & the green line

| Stage | What | Corpus turned green |
|---|---|---|
| **0** | decl surface (routing-agnostic) + entrypoint `host` delegation, keywords, dispatcher hook, classifier, CDZ0401(merged)/0403/0404 | the rejections (CDZ0403, merged-CDZ0401, CDZ0404) + the empty-row `+ 20 22` |
| **1** | Tier-1 tail inline + state threading (unit-state = zero-cost) + under-frame; cross-fn via inlining | unit-state: `Choose.pick→6`, `Get.get→42`, `Scale.by→42`, `Unify/Scope.resolve→5`; stateful fold: `Fresh ×3→2`, `Diag emit/collect→(list 201 210)`; cross-fn: `→42`, `→105`, `→10` (shadow), `→32` (two-handler), `→10` (deep), `→(tuple 0 1)` — **clears the #1 self-host blocker** |
| **1b** | emit-as-function for reused arms (returns `(value,next-state)`) | (none; code-size) |
| **2** | host composition (delegation → boundary call) + the invariant + interposition | `ask.ask→100`, `(+ (ask.ask)(ask.ask))→7`, `Scale.by (ask.ask)→42`, the interpose-and-forward case→7 |
| **3** | effect-context specialization (recursive/runtime-chosen handler) | (none; the moment before a scheduler/recursive effectful fn) |
| **4** | Tier-2 abortive (`block`/`br`) | (none; opportunistic) |
| **5** | Tier-3 defunctionalized general one-shot | (none today; only when a non-tail/captured resume appears — e.g. a scheduler) |

Stage 1 now carries the *whole* self-hosting-relevant surface — tail effects, state folding (the compiler's
`Fresh`/`Diag`/`Unify`), and cross-function resolution by inlining. 2 composes with the host boundary. 3
generalizes cross-function to recursive/runtime-chosen handlers (the bridge to a scheduler). 4–5 are
speculative until a program needs abortive or reified continuations.

---

## 11. What this does NOT touch (so nobody re-derives an envelope by reflex)

- **No change to `runtime.wit` for intra-program effects.** Tiers 0–2 use only the program's own
  locals/stack/scalars/handles; Tier 3 reuses frozen `sum-*`/`arr-*`. `RT_N_IMPORTS` stays put. (A
  host-delegated effect DOES add imported interfaces to the *program's* world — §9 — but that is the
  program's import surface, not `runtime.wit`.)
- **The entry is a plain function (component-abi.md v4).** The v2 `Suspended` result arm is RETRACTED and
  a trap is out-of-band, not a result arm — so the entry is `input -> output`. Intra-program effects never
  reach the boundary and never enter the manifest; a host-delegated effect is a plain imported-function
  call (`host-interface-binding.md` §A Host-Delegated Operation Imports Verbatim). **No suspension/resume
  state is emitted** under any tier — how the host resolves a call (inline / fiber / re-derive) is host
  policy the bytes don't encode.
- **No fuel.** Constitution Amendment 0.7.0 retired compiler-emitted resource accounting; do not add a
  counter. A deep Tier-3 resume chain grows the wasm stack; the host bounds it (a stack-limit trap is a
  defined halt). The eventual optimization is emitting `return_call` (Wasm 3.0 tail call) for resume
  chains — not on the critical path, not emitted today.
- **No native stack-switching.** Rejected for *intra-program* continuations: not in any Wasmtime tier,
  slower than the alternatives, and its opaque native stack can't be re-derived as data — it would break a
  host's re-derivation strategy. (Note wasmtime fibers ARE fine for the *host* boundary — that is the host
  suspending its own call, entirely host-side, invisible to the emitted bytes; different mechanism, no
  compiler involvement.) Revisit stack-switching for intra-program use only as a far-future local-only fast
  path.

---

## 12. Risks / decisions this forces

- **Under-frame (§5)** — the miscompile trap; test nested same-effect handlers first.
- **Classifier conservatism (§3)** — anything uncertain is GeneralOneShot/decline; never "guess tail".
- **effect-op → WIT-import naming (§9) — DECIDED: effect = `interface`, op = `func` in it, world imports
  the interfaces** (matches `runtime.wit`'s `interface heap`). Dissolves same-op-name collisions
  structurally; no flat-string key. Local to the delegation-lowering path + the emitted world.
- **`host` is entrypoint-only** — reject a `host` form outside an entrypoint (authority enters from the
  top; a library never delegates). Decide how "entrypoint" is identified in the seed (the `main` def, or
  any exported entry); until multi-entry lands, treat `main` as the sole entrypoint.
- **Manifest is now computed, not declared** — build it as the union of entrypoint `HostFrame` effect sets,
  replacing the retired `parse_host_imports`/`declared_capabilities` paths; the module's `(meta
  capabilities)` metadata (corpus `11-modules.sexp`) reads from this computed set.
- **Seed exponential-in-nesting** — inlining arms adds IR; prefer Tier-1b for deeply reused arms; watch
  compile time on the first real `Fresh`/`Diag`/`Unify`-heavy program.
- **`Kind::Cont`** — a refinement of `Heap` for Tier-3 type-checking; defer until a non-tail resume
  exists.
- **Multi-shot opt-in surface** — a build-level declared default (not a per-handler annotation); decide
  when Tier 3 is built.
