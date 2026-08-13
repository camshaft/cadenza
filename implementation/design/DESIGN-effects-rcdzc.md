# Design — implementing effects in `rcdzc` (the native reference compiler)

**Author:** compiler engineer. **Audience:** whoever grows `rcdzc` next (task #148).
**Status:** **DESIGN ONLY — nothing landed.** `rcdzc` has **zero** effect surface today (greenfield);
`(meta …)` capabilities still *decline* at `rcdzc/src/resolve.rs:972` and the fold refuses recursive
callees at `rcdzc/src/fold.rs:791`. Where I name a line number it is a landmark at this commit
(2026-07-09), not a promise it won't drift.

> **Not normative, and distinct from the old seed's doc.** The *what* of effects is fixed in
> [capabilities-and-effects.md](../spec/capabilities/capabilities-and-effects.md); the *compilation
> strategy* is fixed in [reference-compiler.md §Effects Are Classified First And Resolved By
> Monomorphization](../spec/architecture/reference-compiler.md); the *operational wasm shapes* + the
> alternatives considered are in [options/effects-model/lowering-to-wasm.md](../options/effects-model/lowering-to-wasm.md).
> [DESIGN-effects-lowering.md](./DESIGN-effects-lowering.md) is the *landed* how-to for the **old**
> `cdz-compiler/src/codegen.rs`; **this** document is the how-to for the **new native `rcdzc`** pipeline
> (`Ast → Hir → typed-Hir → Mir → eval → Lir`). It maps all three onto the concrete `rcdzc` structs, fixes
> the IR node shapes, sequences the work, and — the load-bearing part — designs so that **abortive**
> effects and **captured/stored resumptions** are reachable later without reshaping the IR. It adds no
> requirement and weakens none.

## 0. The one principle everything else follows from

**An effect is an ordinary value, and handler resolution is a reduction on the one compile-time tier.** A
performed operation flows through the pipeline as a first-class value (sibling to `Intrinsic`/`Ctor`/
`FuncRef`), and `handle` is a node the compile-time evaluator (`fold.rs`) *reduces away* — resolving each
performance to a concrete handler arm, rewriting the common case to plain `Mir`. `select` never sees an
effect node in the shipping (tail-resumptive) surface, exactly as it never sees a module record or an
applied constructor today.

This is the structural antidote to the old compiler's brittleness. The old `cdz-compiler` had **no resolved
IR**: it lowered effects by re-walking the unrewritten AST at byte-emit time, so *tail-position*, *handler
resolution*, *state kind*, *recursion*, and *effect-reachability* were each re-derived by several
independent AST walkers that had to agree or silently miscompile (the "3-place rule," the four duplicated
tail-walkers, `format!("{:?}", body)` as a monomorphization cache key, crash-then-cap depth guards,
diagnostics wedged into the emitter). `rcdzc` already solves types once and reads them downstream; effects
extend that discipline rather than reopen the wound. §7 maps each old failure to the structural feature that
prevents it here.

## 1. Monomorphization: the current gap, and why effects depend on closing part of it

Effects and monomorphization are the *same* reduction tier keyed differently
([reference-compiler.md §One Evaluator](../spec/architecture/reference-compiler.md): "handler resolution
is the one compile-time reduction tier applied to the handler context, not a second specialization
mechanism"). So the state of monomorphization in `rcdzc` directly bounds what effects can do.

**What the fold does today** (`fold.rs`, verified):

- Constant folding ✅.
- β-reduction / inline of a **non-recursive** callee **when every argument is a compile-time value** ✅
  (`try_inline`, gated `recursive || arity != args.len() || !all_comptime` at `fold.rs:791`).

**What is absent:**

- **Generic / type monomorphization.** Inference is monomorphic (`infer.rs:11` — no let-generalization);
  each function has *one* signature of shared fresh vars, so a user function used at two concrete types
  would unify those types *with each other*. (Intrinsics and constructors *are* parametric — fresh
  instantiation per use — so the gap is specifically **user-function** polymorphism.)
- **Per-context specialization of a recursive definition.** `recursive_set` marks any call-cycle member
  recursive and `try_inline` refuses it outright. There is no "emit this function once per context it is
  used under" machine.

**What effects need from this, in order:**

| Effect capability | Monomorphization it needs | Have it? |
|---|---|---|
| Tail-resumptive, single function (the fast path) | none | ✅ |
| Cross-function **non-recursive** perform | a **new inline trigger** — "callee reaches an effect an active handler discharges" (nothing to do with constant args) | ❌ (small extension) |
| Cross-function **recursive** perform | specialize a recursive fn **once per handler context**, state threaded through the call boundary | ❌ (new, but isolatable) |
| Abortive / general one-shot **cross function** | the recursive-specialization machine **plus** a non-local-exit calling convention | ❌ (built on the above) |

The headline: **the tail-resumptive shipping surface needs only the new inline trigger** — a small,
well-contained extension of `try_inline`, not a monomorphization project. Recursive specialization (the
corpus's four recursive-walk cases) is the *first real specialization the fold learns*, and building it
correctly builds the exact machine type-monomorphization will later reuse. That is the right order: effects
force us to build the specialization tier on concrete, testable cases before we generalize it to types.

## 2. Modeling effects in the IR (the node shapes)

The shapes below are chosen once, to serve **all four** arm classes (§4) so that abortive and
captured-continuation lowering are later additions, never an IR reshape.

### 2.1 An effect is a closed set of typed operations — like a `SumDef`

```rust
// ty.rs — mirrors SumDef/SumRef exactly (Arc identity = ptr_eq; portable to an integer id later).
pub struct EffectDef { pub name: String, ops: OnceLock<Vec<OpDef>> }
pub struct OpDef     { pub name: String, pub params: Vec<Ty>, pub result: Ty }
pub struct EffectRef(pub Arc<EffectDef>);   // identity by Arc::ptr_eq, as SumRef
```

`(effect E (op f (-> A B)) …)` resolves (in `resolve.rs`, alongside `collect_user_types`) to a compile-time
**record of operation values** bound to `E` — so `E.f` is *ordinary member access* projecting an
`EffectOp` value, and two effects declaring a same-named `op` never collide (the op is reached through its
effect). Duplicate `op` in one declaration → CDZ0201, the same closed-name-set check a sum's variants use.

### 2.2 The effect-operation value — a new first-class leaf

```rust
// Hir / TypedNode / Mir all gain:
EffectOp { def: EffectRef, op: usize }
```

Sibling to `Intrinsic`/`Ctor`/`FuncRef`: carried unchanged through resolve/infer/lower, its type is the
op's `Fn(params, result)`, and it is *only ever reduced in an `Apply`* — a performance `(E.f a)` is
`Apply(EffectOp, [a])`, typed exactly as any application (perform-arg type mismatch → CDZ0201). A bare
`EffectOp` that survives to `select` declines (as a bare `Intrinsic`/`Ctor` does). Nothing is privileged by
name.

### 2.3 `handle`, the arm, and `resume` — designed for all four classes

```rust
// Hir/Typed/Mir node:
Handle {
    init:  Box<_>,                 // the seed state, evaluated where the handle is installed
    arms:  Vec<HandleArm>,
    body:  Box<_>,                 // the handled sub-computation; the handle's value is its value
}
struct HandleArm {
    op:      (EffectRef, usize),   // the operation this arm discharges (must be declared → else CDZ0403)
    params:  Vec<u32>,             // the op's parameters, bound in the arm body
    state:   u32,                  // the CURRENT state, bound in the arm body (the left-fold accumulator)
    body:    Box<_>,               // contains Resume nodes (tail case) OR none (abortive) OR a captured k
}

// The resumption. Modeled as a NODE (not a fold-only rewrite marker) so it can survive to a
// general/abortive lowering, and so a captured continuation is representable.
Resume { value: Box<_>, next_state: Box<_> }   // (resume value next-state)
```

The two future-critical shape decisions, made now:

1. **`Resume` is a real node, not just a fold-time textual substitution.** In the tail-resumptive path the
   fold *does* rewrite it away (`Resume{v, s'}` → `v`, thread `s'`). But because it is a node, an abortive
   arm (no `Resume` at all) and a general arm (a `Resume` not in tail position) are representable and
   *classifiable structurally* rather than by walking raw syntax. This is what lets §4's classifier be one
   fold over the IR, not the old compiler's four hand-walkers.

2. **The continuation is, in the general case, a first-class value.** The current surface spells resume as
   the 2-arg `(resume value next-state)` with an *implicit* continuation. To reach captured/stored
   resumptions (§4.4) we reserve the more general form: an arm may bind the continuation `k` as a value
   (`ctl`-style), and `(resume k v)` / storing `k` in a list/map are ordinary applications/values of a
   `Ty::Cont` type. The tail surface is the *classifier-recognized special case* of this — `k` used exactly
   once in tail position. We do **not** build the `ctl` surface now, but `Resume`-as-node + a reserved
   `Ty::Cont` mean adding it is a new arm class, not an IR migration.

### 2.4 `host` delegation

```rust
Host { effects: Vec<EffectRef>, body: Box<_> }   // an ENTRYPOINT-only delegation
```

`gen_host`'s twin of `handle`: it routes its listed effects to the component boundary. Admitted only at an
entrypoint (a non-entrypoint `host` → reject). Its manifest contribution is handled entirely in
`serialize`/`component` (§6) — no pass above the serializer touches wasm encoding.

## 3. Where the work happens: the fold becomes handler-context-aware

Handler resolution is a reduction, so it lives in `eval` (`fold.rs`), which gains a **handler-context
stack** threaded as it descends (today `fold` is context-free bottom-up; it becomes context-carrying —
a contained change). Recommended: put the effect-specific helpers in a new `effects.rs` the fold delegates
to, so "resolve/classify/lower an effect" is one concern in one place (the discipline the old compiler
violated by smearing it across ~six functions).

The fold, on descent:

- `Handle{init, arms, body}` → fold `init`; push a frame of classified arms (seeded with the folded init
  state) onto the context; fold `body`; pop. Each arm carries its **definition-site depth** (the context
  length at the handle) — the *under-frame*.
- `Apply(EffectOp{E,op}, args)` (a perform) → resolve `E.op` against the context **top-down** (nearest
  enclosing handler wins — dynamic extent). The result is a single concrete arm and its class, **a
  compile-time constant** — no runtime search. Lower per §4.
- `Call{f, args}` where `f` reaches an effect an active handler discharges → **inline `f` into the handled
  region** (the new trigger; reuses the existing α-rename+substitute inline path). A **recursive** such `f`
  cannot be inlined → **specialize** it (§4.3). A perform with no enclosing handler *and* no enclosing host
  delegation at the entrypoint top → **CDZ0401**.

**The under-frame rule** (the old compiler's #1 verified-subtle landmine): an arm body's *own* performs
resolve against the context enclosing the arm's **definition**, not the perform site — so a forwarding
handler re-performs outward, never into itself. Here that is *not* a mutate-and-restore of a live stack
(the old `split_off`/`extend` dance, exception-unsafe); it is: **fold each arm body against the context
truncated to the arm's recorded definition-site depth.** Definition-site context is an immutable property of
the arm, read — not spliced. Test a nested same-effect handler before anything else.

## 4. The four arm classes as a cost ladder

Classification is one fold over the IR (`Resume` count + tail-position of the single `Resume`), computed
once, **conservative toward the more expensive class** (an unprovable arm is general, never mis-lowered as
tail — a tail misclassification silently drops post-resume work, the one miscompile we never ship). An arm's
class is the least-upper-bound over its control paths; a runtime branch never changes it.

The ladder is deliberately layered so each rung reuses the one below:

### 4.1 Tail-resumptive → plain code (E1; the entire shipping surface)

Arm resumes **exactly once, in tail position**. The fold:
1. Binds `pᵢ ↦ argᵢ` and `state ↦ current-state`.
2. Rewrites every tail `Resume{v, s'}` → `v`, threading `s'` as the state the rest of the handled region
   sees.
3. Splices the rewritten body at the perform site under the **under-frame**.

State threads as an **explicit value** (a `let` / an extra threaded binding), never a mutable wasm local
with a re-encoded kind tag (the old `@state-local` node that forced the 3-place rule). Unit-state is the
degenerate zero-cost case (`Ty::Unit` occupies no slot). No continuation object, no handler stack, no
evidence vector. **`select` sees only plain `Mir`.** Covers corpus Groups 1–3 (11 cases) once §4.3's inline
trigger lands.

> **Known limitation — handler-dispatch-count ceiling (Finding #24, deferred).** Because the threaded state
> is spliced as an *explicit value* (above), a handler whose arm builds its next-state as a **compound of the
> prior state** (e.g. a `Map`-state handler resuming `(Map.insert prior k v)`) grows that state EXPRESSION by
> one arm per dispatch, and the fold's `deep_fresh_copy` of the threaded state re-materializes the whole
> accumulated expression each dispatch → the emitted Core is **O(kᴺ)** in the number `N` of sequential
> dispatches to one handler. This is a compile-time SIZE blow-up, not a miscompile: all backends emit the
> correct value, and wasm tolerates the size, but a deep async **rust** emit can exceed rustc's recursion
> limit (SIGSEGV) at large `N`. In practice only a handler driven through **many** sequential same-effect
> performs with a compound accumulating state hits this; a bounded/typical reducer is fine. The root is the
> per-dispatch copy of the shared state subtree (`effects.rs` perform-arm reify); a within-fold fix is
> exhausted (the substituted state loses node-identity through `beta_reduce`'s structural-copy paths, so it
> cannot be shared back at the reify site), so the durable fix is a **Core-level threaded-state primitive**
> (an explicit shared binding the fold emits and lowering understands, keeping the chain linear) — a funded
> feature to be built when a real reducer actually reaches this ceiling, not to satisfy the synthetic probe.

### 4.2 Abortive → non-local exit carrying a value (E4; *you asked for this*)

Arm **never resumes**: its body's value *becomes the handle's value*, discarding the rest of the
sub-computation. This is a typed early-exit / "bail and catch at the top":

```
(handle Fail 0                              ; effect named in the head; 0 = default if we bail
  ((fail (msg) s msg))                       ; arm op written bare; no resume → abortive: yields `msg`
  (do (check-a) (Fail.fail 7) (check-b)))   ; performing Fail.fail abandons the body, handle = 7
```

Lowering, **within one function**: emit `body` inside a wasm `block` whose result type is the handle's
result type; lower a perform of an abortive op to `br` to that block's end carrying the arm's value. No
capture, no continuation. Represented by a small structured-control `Mir` pair the fold produces and
`select` maps to `block`/`br` (`Mir::Block{result_ty, body}` + `Mir::Break{value}`) — so `select` emits flat
instructions from an *explicit* control node and never resolves an effect. This keeps the flat rung flat
(control it cannot express is an explicit node, not a sniffed shape).

**Cross-function abortive** (perform deep, handle at the top — the "handle it at the top" you want)
introduces the **non-local-exit calling convention**: a function specialized under an abortive context
returns a discriminated `normal | aborted(value)` and each caller on the path propagates an `aborted` up to
the handler's frame, which yields it as the handle's value. This reuses §4.3's specialization keying (a
function is specialized per handler context; an abortive context selects this convention). This is the
"early return all the way up the call stack" — for abortive it needs **only** propagate-up, **no** frame
capture, so it is strictly cheaper than §4.4 and is the natural first build of the non-local-exit machinery.

### 4.3 Recursive-function specialization (E3; the monomorphization gap, made concrete)

A **recursive** effectful function cannot be inlined (it would not terminate). Emit it **once per handler
context it is called under** (`gen_specialized_call`'s clean re-build): each enclosing handler's state
becomes a hidden trailing parameter, threading each context's state **through the call boundary**, never a
global (a global clobbers on nesting/re-entry). Self- and mutual recursion and nested handler states
follow. Covers corpus Group 4 (4 cases: recursive walks pulling `Fresh`/`Diag`/`Countdown` state).

> **🔑 2026-07-12 finding (verified against all four executing corpus cases): the general form wants a
> multi-value return `f#ctx(params…, s_in…) -> (result, s_out…)`, but NO corpus case needs it.** Every
> corpus recursive-effectful function has a **downward-threaded, single-return** shape: the handler state
> flows *into* the recursive self-call as a trailing parameter, and the result is a plain scalar — the
> final state is never carried back up (the handle's value is the *body's* value, not the accumulated
> state). So the shipping realization is just `f#ctx(params…, s_in…) -> result` (no multi-value, no backend
> change): rewrite each perform to its arm's resume *value* (a function of `s_in`, via the same
> `beta_reduce` substitution the tail fold uses) and rewrite the self-call `(f …)` to `(f#ctx …,
> <threaded next-state>)`. Concretely — `loop#ctx(s) = (if (= s 0) 0 (+ 1 (loop#ctx (- s 1))))` (countdown);
> `sum-down#ctx(s) = let i = s in (if (= i 0) 0 (+ i (sum-down#ctx (- s 1))))` (range-sum); two nested
> handlers → two trailing params. The `s_out` multi-value return is only needed when a caller observes state
> *after* a recursive callee returns (no corpus case does) — defer it until one exists. This makes E3 a
> source-to-source specialization + a synthesized `db.defs` entry reached through the existing
> `Core::Call`+`layout` reachability, NOT a backend/ABI change.

Two disciplines the old compiler learned the hard way, adopted from the start:
- **The context key is a resolved handler-context identity** (interned `EffectRef` + arm identities), **not**
  `format!("{:?}", arm.body)`. No stringly-typed syntax fingerprint.
- **An unbounded handler context** (a recursion installing a fresh `handle` per call → the corpus's one
  Group-5 decline, `→100`) is a **computed** decline — the context set does not close — not a
  crash-then-cap on a magic depth number, and the bound must hold for the **smallest target** the compiler
  runs on (a native-vs-wasm stack differential already bit the old caps).

This is the substrate §4.2 (cross-function) and §4.4 build on, and the machine §1's type-monomorphization
reuses.

### 4.4 General one-shot & captured/stored resumptions (E5; *the future you want kept open*)

Trigger: a `Resume` **not in tail position**, or the continuation **captured as a value** (stored in a
list/map, resumed later). This is the powerful case you called out — and its cost is real: the handler arm
**cannot be inlined at the perform site**, and the path from perform to handle must be compiled to
*suspend-and-resume*, i.e. the non-local-exit convention of §4.2 **plus** reifying the continuation.

Design (deferred to build, kept reachable now):
- Reify the delimited region perform→handler as a **defunctionalized frame chain on the frozen value-heap
  prefix**: a frame is `sum-new(site-disc, arr-of-captured-locals)`; `k` is the frame handle — an ordinary
  heap value of the reserved `Ty::Cont`, so it can be **stored in a list/map and resumed later**.
- `resume k v` = `apply(k, v)`, where `apply` is one compiler-emitted `br_table` dispatcher (a fixed helper
  — control the flat rung can't express is a fixed helper, not a new instruction). **Envelope-neutral**: no
  new WIT op (frames are `sum-new` over `arr`; `apply` is in-program).
- **One-shot** consumes the chain once (RC-reclaimed). **Multi-shot** (resume `k` more than once — the
  store-and-replay-many case) copies the frame chain per resume; a **per-build opt-in** (a second resume
  under the default is a compile-time rejection, not a runtime path).

**What we build now to keep this open, at zero cost to the shipping surface:**
1. `Resume` is a node (§2.3) → a non-tail resume is representable and *classified*, not mis-lowered.
2. `Ty::Cont` is reserved in the type enum (no runtime rep yet; `core_valtype` = the heap-handle `I32`
   when built) → a captured continuation has a type to be.
3. The effect **row is inferred per function** (§6) → E5 knows *which* call paths reach a general-class
   handler and must be compiled suspend-able; the rest stay plain.
4. The non-local-exit convention is introduced by §4.2 (abortive) → E5 adds frame capture *on top*, not a
   new control mechanism.

Until built, a general-class arm is a **clean decline** (never a valid-but-trapping component). The
**host-composition invariant** holds by construction: a reified (Tier-3) continuation **must not span a host
call** (a re-deriving host cannot reconstruct a chain of run-local heap handles) — statically checkable from
the same classifier, and today's corpus satisfies it automatically.

### The ladder, summarized

| Class | Resumes | Mechanism | Cross-fn cost | Rung |
|---|---|---|---|---|
| Tail-resumptive | once, tail | inline + rewrite `Resume`→value; thread state as a value | inline (or E3 specialize if recursive) | E1 / E3 |
| Abortive | never | `block`/`br` within-fn; non-local-exit up-stack cross-fn | propagate-up, **no** capture | E4 |
| General one-shot | once, non-tail / captured | defunctionalized frames; `resume`=`apply(k,v)`; `k` storable | non-local-exit **+** frame capture | E5 |
| Multi-shot | >once | general + copy frame chain per resume | as general, ×copies | E5, opt-in |

## 5. Effect rows and the manifest

- **Row inference** is a fixpoint over the call graph (the same shape as the existing `recursive_set`): a
  function's row = the effects its own performs reach ∪ its callees' rows. Cheap, materialized once
  (bounded-in-nesting per the cost discipline). This is also the input §4.4 reads to decide which paths need
  suspend-able compilation.
- **Discharge removes a label**: an effect a nearer `handle` discharges leaves the row of the wrapped
  computation → it never reaches a `host` delegation and never enters the manifest (this is why
  interposition is free).
- **The manifest is the escaping row** = the entrypoint's delegated-and-reached effects. A delegated effect
  never reached → **CDZ0404** (latent authority). Purity is the empty row.
- **Boundary encoding lives in `serialize`/`component` alone**: an effect is a WIT **interface** (a
  component instance), an op a **function** in it — a dotted `E.op` is never a top-level extern name (the
  component model forbids the dot; the old compiler learned this the hard way). No pass above the serializer
  writes an encoding byte.

## 6. Diagnostics (register in `diag.rs::Code`)

| Code | Meaning | Where produced |
|---|---|---|
| **CDZ0401** | an effect reached with **neither** an enclosing handler **nor** an entrypoint delegation — the merged "no home" check (subsumes the retired CDZ0402) | fold, at an unresolved perform |
| **CDZ0403** | a handler arm names an operation its effect does not declare (closed-set violation) | resolve/infer of `handle` |
| **CDZ0404** | a `host` delegation names an effect the body never reaches (latent authority) | after row inference |
| (CDZ0201) | perform-arg / resume-value type mismatch, duplicate op name | infer (ordinary type error) |
| (CDZ0101) | unbound name in a resume-state position | resolve (ordinary scope check) |

Every one is produced in a resolution/checking pass over typed IR — **never wedged into byte emission** (the
old compiler ran CDZ0201/0101 inside `emit_handler_arm` because the emit path unwrapped subterms away before
they could be checked; with a resolved IR the checks are uniform and once).

## 7. Anti-brittleness ledger — old failure → structural prevention here

| Old-seed brittleness (file:line in `cdz-compiler/src/codegen.rs`) | Prevented here by |
|---|---|
| Tail-position re-walked in **four** must-agree functions (`resume_in_tail`/`for_each_tail_resume_*`/`unwrap_tail_resume`, ~14744–14872) | `Resume` is an IR node; classification is **one** fold over it (§2.3, §4) |
| **3-place rule**: a form's kind/shape recomputed in emit + `infer_list` + `shape_of_list`; `@state-local` synthetic node with an integer kind tag | solve-once/read-downstream: kind read off the solved `Ty`; state threaded as a typed value, not a tagged synthetic node (§4.1) |
| "Emit an arm" smeared across ~6 fns with mutable `split_off`/`extend` of the live router stack (exception-unsafe; the design's own #1 landmine) | under-frame is an **immutable recorded depth**, read; effect lowering is one delegated concern (§3) |
| Monomorphization cache keyed on `format!("{:?}", arm.body)` (10278) | context key is a **resolved handler-context identity** (§4.3) |
| Two crash-then-cap guards (`MAX_HANDLER_CONTEXT_DEPTH=8`, `MAX_SPECS_PER_FN=64`), non-portable | a **computed** decline (context set doesn't close), bound holding for the smallest target (§4.3) |
| Manifest built from string scans + re-fabricated `elems`; hand-computed `spec_wasm_index`/`call_base` offset contract | manifest from **resolved** effect types + rows; indices assigned by the layout, referenced symbolically (§5, `layout.rs`) |
| Diagnostics (CDZ0201/0101) run **inside** byte emission | produced in a checking pass over typed IR (§6) |
| Latent stateful-branch cliff: `if`/`match` tail in a *stateful* arm silently dropped state threading | state threading is a **total** IR rewrite over the classified arm (§4.1), not a two-form hand-walk |
| Host-delegation × recursive-effect had to be **turned off** (incomplete reconstruction) | one specialization tier threads *all* enclosing contexts uniformly (§4.3) |

*Keep* from the old compiler: the classify-first strategy itself (sound; it is the corpus's whole surface),
and the `Rc`-shared capture env that turned O(2^depth) into O(depth) — fold into whatever binding the inline
path uses.

## 8. Staging (each stage names the corpus cases it turns green)

Corpus: `spec/semantics/14-effects-and-handlers.sexp` (30) + `04-capabilities.sexp` (7).

- **E0 — surface + rows + rejections (no lowering).** Parse `(effect …)`/`handle`/`host`; add `EffectOp`
  leaf + `EffectRef`/`EffectDef`; infer effect rows; wire **CDZ0401/0403/0404**; reserve `Ty::Cont`.
  *Green:* the 11 rejection cases + the 2 pure/empty-row cases; effect constructs stop misfiring the
  syntactic dispatch.
- **E1 — tail-resumptive in the fold (the core; the #1 self-host unblock).** Handler-context stack;
  resolve+classify; rewrite tail `Resume`→value; thread state as a value; the **new inline trigger** for
  non-recursive effectful callees; the **under-frame** (test a nested same-effect handler first).
  *Green:* Groups 1–3 — `Choose.pick→6`, `Get.get→42`, two-effects→5, `Fresh×3→2`, `Diag→(list 201 210)`,
  and the 6 cross-function non-recursive cases. **Clears the top self-host blocker (#148).**
- **E2 — host delegation.** `Host` node → boundary import (effect=interface, op=func) in `serialize`;
  manifest=union; the deterministic `(host-responses …)` model; the host-composition invariant.
  *Green:* the 8 Group-6 cases (incl. interpose-and-forward → 7).
- **E3 — recursive-effectful specialization (builds the monomorphization tier). ✅ DONE.** Specialize a
  recursive effectful fn once per handler context; state as trailing params (single-return — the
  multi-value return is unneeded per the verified §4.3 finding); computed unbounded-context decline.
  *Green:* Group 4 (countdown→3, range-sum→6, recursive `Diag` list walk→3 [E3g: empty-list seed typed
  from arm op params], two-nested-states→30 [E3h: state threaded as a VECTOR of slots, nested contexts
  MERGED when a recursive callee spans both]); the Group-5 unbounded case declines cleanly (→100 stays a
  decline, as designed). NO multi-value return was needed (downward-threaded single-return, verified).
- **E4 — abortive (you asked for it).** `Mir::Block`/`Break` + within-function `block`/`br`; then the
  cross-function **non-local-exit** convention (propagate-up, no capture) on E3's specialization keying.
  *Green:* any exception-shaped effect; unblocks "bail and catch at the top." (No corpus case exercises it
  today — build against fresh cases.)
- **E5 — general one-shot + captured resumptions (the future, kept open by §4.4).** Defunctionalized frames
  on the frozen prefix; `apply` `br_table` helper; `Ty::Cont` gets its heap rep; `k` storable in list/map;
  multi-shot behind a per-build opt-in. Reuses E4's non-local-exit + adds frame capture. Until then a
  general arm is a clean decline.

## 9. Decisions deferred (call them out; don't pre-commit wording)

- **`fun` vs `ctl` surface.** Whether a handler must *declare* it captures the continuation (so a stray
  non-tail resume is an explicit local error, not a silent reclass to the expensive tier). Recommended when
  E5 lands; `Resume`-as-node makes it additive.
- **Where the effect row sits relative to annotation.** Inference is mandatory (the escaping row); the
  opt-in annotation layer (`type-system.md`) is checked against it. Fold in with the row fixpoint.
- **ANF.** Deferred per the spike verdict — effect-capture (E5 only) works with on-demand naming on the
  nested tree; ANF is a 10–20% ergonomics win, not a blocker, and currently breaks literal detection. Revisit
  as a `Mir→Lir` cleanup after E5, not before.
- **`Ty::Cont` runtime representation.** Reserved now; its heap layout (a frame-chain handle) is fixed at E5.
