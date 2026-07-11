# ANF step 2 — runtime functions + recursion (`Core::Call`, reachability, recursion typing)

**Status (updated 2026-07-11):** A1 landed `@376acc3` (def_scheme), B1 landed `@6fbe023` (Core::Call +
reachability — ANNOTATED recursive functions run end-to-end). **A2 remains** (unannotated-parameter
inference — the connected def-body solve) as the deferred miscompile-prone piece; it is what ungates
the corpus's unannotated recursive cases. Written 2026-07-11 on branch `rcdzc-rewrite`. Companion to
`scope-next/implementation/DESIGN-anf-rewrite.md` (§7 recommends introducing ANF *with* runtime
functions). Line numbers were landmarks at `073efda`.

> **B1 outcome:** annotated `sum-to`/`fac`/`all-lt`/`all-ge`/mutual `is-even`/`is-odd` all compile and
> run under wasmtime; overflow traps across call frames; an unannotated recursive def declines cleanly.
> The "recursion fixpoint" (A) turned out UNNEEDED for annotated defs (they type by absorption — §2.5),
> so B1 shipped on the safe path. A2 is purely the unannotated-param solve.

> **The one-line finding.** In the columns-rewrite `rcdzc`, a `Core::Call` is *inseparable from
> recursion*: every non-recursive user call already FOLDS (β-reduces to a normal form at compile time
> via `eval::apply_lambda`), so the ONLY program that forces a runtime call is a **recursive** one —
> and a recursive function has **no type today** (it can't β-reduce, and `infer` has no def-level
> scheme / recursion fixpoint). So this increment is really *two* things bolted together: (A) a
> Stage-2 HM extension — give a `def` a **scheme** solved with a **recursion fixpoint** — and (B) the
> Stage-3/backend plumbing — `Core::Call`, reachability past exports-only, call/local ABI. (A) is the
> risky part; the memory flags recursion typing as *the* miscompile-prone class
> (`inference-plan-learn-from-seed-coarse-kind-mistakes`: "every seed inference ask = ONE bug,
> order-dependent placeholder-vs-concrete unification"). Do (A) behind its own gate before (B).

---

## 1. Why a `Core::Call` only ever arises from recursion (measured, not assumed)

- `lower.rs:95` — an `Apply` whose head is a lambda β-reduces (`eval::apply_lambda`) and the reduced
  body is lowered. A top-level `def` resolves to `Resolved::Lambda` (`resolve.rs:200`), so `(f 3)`
  inlines `f`'s body with `3` substituted. This is the monomorphization tier; it terminates for any
  non-recursive callee.
- `eval::is_recursive` (`eval.rs:284`) is a static call-graph DFS; `apply_lambda` (`eval.rs:223`)
  DECLINES a recursive callee *before* inlining (else it inlines without end / explodes exponentially
  on a branching body). That decline is what makes `sum-to(3)` a `todo` today (verified:
  `xtask gate --case "recursive def computes"` → `declined`).
- Therefore: to compile ANY recursive corpus function (21 lines cite `sum-to`/`all-lt`/`all-ge`/
  factorial in `09-functions.sexp`), `apply_lambda`'s recursive decline must become a **`Core::Call`
  to a separately-emitted function** instead of an inline — and that callee must be TYPED (a wasm
  function needs param + result valtypes), which is where (A) comes in.

Everything else that *could* want a call (a shared non-recursive helper, a higher-order function)
still folds correctly today, so it does not force this work. Do NOT add a `Core::Call` for the
non-recursive case — that would regress byte-identity (a folded constant would become a call) for no
corpus gain. **The rule: β-reduce when you can (non-recursive), emit a `Core::Call` only when you
can't (recursive).**

## 2. The two halves

### (A) Recursion typing — a def-level scheme with a fixpoint (Stage-2 HM extension)

Today `infer` is purely per-node, memoized, NO `Subst` threaded across a body (`infer.rs:33`), and a
`def` has no scheme — a call types only via β-reduction of the *reduced* body. A recursive def can't
reduce, so `type_of` of a `(sum-to n)` call currently returns `Any` (`apply_type`'s recursion guard,
`infer.rs:169`). `Any` is why it doesn't cascade a spurious error — but it also means no width/no
valtype, so it can't cross to a wasm signature.

**What's needed:** compute a **scheme for a `def`** — `def_scheme(db, def_index) -> Scheme` — by the
standard monomorphic-recursion fixpoint:

1. Give the def a fresh result variable `r` and each parameter a fresh variable `p_i` (an annotated
   param uses its annotation type instead of a fresh var — the annotation is a constraint).
2. **Seed the environment with the def's own signature** `(p_0, …, p_n) -> r` BEFORE typing the body,
   so a self-call inside the body unifies against this provisional signature (this is the fixpoint —
   a recursive call is typed against the very signature being solved). Mutual recursion seeds the
   whole strongly-connected component together.
3. Type the body under that environment, threading ONE `Subst` (this is the departure from per-node:
   a def body's inference is one connected solve, because a self-call's argument constrains a
   parameter and the body's value constrains `r`). Unify the body's type into `r`.
4. Apply the final `Subst`; the def's scheme is `(p_0…p_n) -> r` grounded. An UNDETERMINED variable
   that survives (a parameter no call and no body use constrained) is a **rejection**, not a defaulted
   type (build-order Stage 2 "done when": "an expression whose type is left undetermined causes a
   rejection, not a defaulted type") — EXCEPT a bare literal's deferred width, which grounds to the
   default as it does everywhere.

**Storage.** The scheme is a fact about a def occurrence → cache it keyed by the def's body `StructId`
(like `db.recursive` / `db.build_cache`), or add a `schemes: HashMap<usize, Scheme>` on `Db`. It is
NOT a per-node type column entry — it is the def's generalized signature. `type_of` of a self-call or
a cross-def call reads `def_scheme`, instantiates it, unifies args (the existing `apply_type` shape,
but reading the def scheme instead of β-reducing).

**Order-independence (the memory's non-negotiable).** The fixpoint MUST reach the same solution
regardless of which node is demanded first. The seeded-signature-before-body approach gives this: the
provisional signature is a set of fresh vars, and unification is order-independent (`unify.rs`), so a
self-call seen before or after the body's base case reaches the same `Subst`. TEST THIS EXPLICITLY
(build-order "done when": forward ref, self ref, mutual recursion type identically regardless of visit
order) — it is the single highest-value test in this increment.

**Watch-outs (from the coarse-kind post-mortem + the seed's ask-14/…/77 family):**
- Do NOT re-derive a type at emit. The scheme is solved once into the cache; select reads it.
- A still-unsolved self-call must not let branch ORDER decide the type. `09-functions.sexp:225`
  spells out the exact hazard: `(if guard (recurse …) false)` vs `(if guard false (recurse …))` — a
  Bool-literal branch pins the result to Bool, and BOTH orders must type identically. The fixpoint +
  order-independent unify handles this iff the join of an `if`'s branches unifies (not the current
  `Ty::join` heuristic — a recursive branch is `Var`/`Any` and must UNIFY with the literal branch, not
  be `join`ed). Check `infer`'s `if` arm: it currently `join`s branch types (`infer.rs:89`); for a
  recursive body the self-call branch is `Any`, so `join` yields the other branch — which happens to
  be right for Bool but is a heuristic, not a solve. The fixpoint's `Subst` should unify the branches
  and read the result, making it principled.

### (B) `Core::Call` + reachability + ABI (Stage-3/backend plumbing)

Once a recursive def has a scheme:

- **`core.rs`:** add `Core::Call { callee: usize, args: Vec<StructId> }` (callee = a `db.defs` index;
  args = operand occurrences, each an atom after A-normalization — see general operand ANF below).
- **`lower.rs`:** in the `Apply`-lambda arm (`lower.rs:95`), when `is_recursive` (or, more precisely,
  when `apply_lambda` would decline for recursion), emit `Core::Call { callee, args }` instead of the
  decline. The callee index is resolved from the head (`resolve.rs` already resolves a def-name to a
  `Lambda` carrying the def's params/body; map back to the def index via `db.def_by_name` or carry the
  index). Non-recursive calls STILL β-reduce (unchanged).
- **General operand A-normalization (the ANF piece step 1 deferred).** A call's argument must be an
  atom (`ConstInt`/`ConstBool`/`LocalRef`/`Param`). A compound argument (`(sum-to (+ n -1))` — the arg
  is `(+ n -1)`) must be hoisted to a `Core::Let` binding and passed as a `LocalRef`. This is the
  first admin binding with NO source occurrence, so it needs the fresh-id space step 1 deferred:
  synthesize a `StructId` via `Db::push_*` above `user_node_count` (the `sanitize_origin` boundary at
  `compile.rs:98` already drops non-user origins from diagnostics — the machinery is in place). A
  synthesized binder's type is its value's type (read the value's `type_of`). Keep it minimal: hoist
  ONLY non-atomic call arguments; a constant/param/localref arg passes directly.
- **`layout.rs`:** grow `order` past exports-only. After seeding with exports (unchanged), walk the
  reachable call graph: for each def in `order`, find the `Core::Call` callees in its body and append
  any not already in `order`. `abs(def)` already maps a def to its emission index (`layout.rs:58`);
  keep base 0 (no runtime import in this slice). A recursive callee that is also an export appears
  once (dedup by `contains`).
- **`select.rs`:** `Core::Call { callee, args }` → emit each arg (atoms → `local.get`/`const`), then
  `Lir::Call(abs_index)`. The callee's params occupy its own slots 0..n as usual. Add `Lir::Call(u32)`
  + opcode `0x10` + serialize arm.
- **`serialize.rs`:** a non-export reachable function still needs a functype + code entry (it already
  iterates `layout.order`, `mod.rs:34`); it just won't appear in the export section. Its param/result
  valtypes come from the def scheme (A). `mod.rs:36` currently only finds params for EXPORT defs
  (`layout.exports.iter().find`); a non-export callee needs its params from the def scheme too — thread
  the scheme's param types into `select_function`.
- **`compile.rs`:** `collect_reached_poisons` — `Core::Call` args are unconditionally evaluated,
  descend into each; the callee's body faults surface when IT is collected (it's in `order`). NOTE:
  `collect_faults` currently only trap-walks NULLARY def bodies (`compile.rs:163`) — a recursive def
  with params is not nullary, so its body's traps are NOT collected standalone (correct — its params
  are unbound until called; a provable trap inside surfaces at a constant call site via the fold, or
  is a runtime trap). Keep that; just ensure the recursive def is TYPE-checked (it is — `type_errors`
  runs on every body).

## 2.5 KEY FINDING from A1 (2026-07-11) — annotated recursion needs NO fixpoint

Sub-increment A1 landed `def_scheme` and pinned a fact that materially shrinks the risk: **an
ANNOTATED recursive def types correctly WITHOUT an explicit recursion fixpoint.** A self-call returns
`Any` (the recursion guard in `apply_type`/`apply_lambda`), and `Any` is ABSORBED by unification/join
with the concrete parts of the body — the base case (`0`) and the non-recursive arm (`(+ n …)`) pin
the result to `Int64`. Verified: `def_scheme((def (sum-to (: n Int64)) (if (= n 0) 0 (+ n (sum-to (+ n
-1)))))) = Int64 -> Int64`, and it's order-independent (the self-call is `Any` regardless of visit
order; a concrete branch always determines the type). An explicit fixpoint is only needed when NO
concrete part pins the result — which terminating monomorphic recursion cannot do (a base case must
exist and pins it).

**Consequence for scope:** (A2) "recursion fixpoint" is NOT needed for annotated recursive functions.
The residual A2 work is purely UNANNOTATED-PARAMETER inference (`(def (sum-to n) …)` — determine `n`
from its uses `(= n 0)`/`(+ n …)` + the call-site argument), which is the connected def-body solve —
still the miscompile-prone part, still deferred. **B1 can target ANNOTATED recursive functions right
now** (verified: an annotated `sum-to`'s ONLY blocker is the lower-time recursion decline at
`lower.rs:106`; its type is already correct). So the near-term path is: B1 over annotated recursive
defs (low risk — types are determined), then A2 (unannotated inference) as a separate supervised step
to ungate the corpus's unannotated `sum-to`/`all-lt`/`all-ge`.

## 3. Recommended sub-increments (each its own commit + gate)

1. **(A1) `def_scheme` for a NON-recursive def, behind the existing β-reduce path.** Compute a def's
   scheme and TEST it agrees with what β-reduction produces for a few calls (a cross-check: the scheme
   instantiated + args unified == the reduced body's type). No behavior change yet (calls still
   inline). This lands the scheme machinery + the order-independence tests on safe ground. Gate
   unchanged.
2. **(A2) recursion fixpoint in `def_scheme`.** Seed the signature, type the body, solve. Now
   `type_of` of a recursive call returns a real type instead of `Any`. Still no `Core::Call` — a
   recursive call still DECLINES at lower (so gate unchanged), but its TYPE is now known. Test:
   `type_of` of `(sum-to 3)` is `Int64`; the two `all-lt`/`all-ge` order cases type as `Bool`
   identically; a mutual-recursion pair types consistently; an undetermined signature REJECTS.
3. **(B1) `Core::Call` + `Lir::Call` + reachability + ABI**, non-atomic-arg hoisting via synthesized
   `Core::Let`. Now `sum-to(3)` compiles and runs. Gate: the recursive corpus cases flip todo→pass
   (target: `sum-to`, `all-lt`, `all-ge`, factorial-with-if; the match-based ones need match = a later
   step). Run each under wasmtime.
4. **(B2) mutual recursion + a shared non-recursive helper emitted as a call** IF a corpus case needs
   a helper NOT to inline (unlikely — non-recursive still inlines; only add if a case regresses on
   code size or a fold depth limit). Probably skip.

## 4. What does NOT belong here (keep scope honest)

- `match` — the match-based recursive cases (`sum-to` with `(match n (0 0) (_ …))`, factorial) need
  the match engine. That is step 3 (records/tuples → match → sums). The `if`-based recursive cases are
  the target here.
- Higher-order runtime functions / closures that ESCAPE (returned, stored) — still fold or decline;
  runtime closures are a later stage (the memory's "runtime closures = Increment B, deferred").
- Effects — Stage 6, unchanged.

## 5. Verification

- `cargo test -p rcdzc` — new unit tests for `def_scheme` (order-independence trio: forward/self/mutual;
  undetermined→reject), `Core::Call` lowering, reachability order.
- Behavior: `sum-to(3)=6`, `all-lt(0,3,5)=true→1`, `all-ge(0,3,0)=true→1`, an `if`-based factorial,
  run under wasmtime.
- Gate `xtask gate --check`: the `if`-based recursive cases flip todo→pass (a POSITIVE baseline delta,
  re-saved additively); nothing regresses. Byte-identity: a non-recursive program is UNCHANGED (calls
  still fold) — verify a scalar/arith program's bytes are byte-identical to before.
- fmt + clippy clean.

## 6. Risk register

- **HIGHEST: recursion typing order-dependence.** Mitigate: sub-increment A2 in isolation with the
  order-independence trio as the gate; do not proceed to B until it's green.
- **Synthesized-id hygiene** (non-atomic arg hoisting): a synthesized binder must never leak to a
  diagnostic (the `sanitize_origin` boundary handles it) and must get a correct type + slot. Test a
  call with a compound arg (`(sum-to (+ n -1))`) specifically.
- **Byte regression** for non-recursive programs: guard by keeping the β-reduce path the default and
  only emitting `Core::Call` for the recursion-declined case; assert byte-identity on a non-recursive
  fixture.
- **`layout.order` growth correctness**: a reachable callee appended must dedup and its `abs` index
  must match the call site's emitted index — test a 2-function module's indices.
