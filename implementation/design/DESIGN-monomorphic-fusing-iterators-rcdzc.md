# DESIGN: Monomorphic, generic, FUSING iterators (Rust-style) — rcdzc

**Status:** design sketch for operator review (v-iterators, 2026-07-18).
**Charter:** operator directive (relayed via concierge) — the iterator family must move from
enum-tagged runtime dispatch to a MONOMORPHIC, GENERIC, FUSING design: `map().filter().fold()`
should compile to a single fused loop, "as if you wrote the whole chain of functions by hand,"
giving the compiler maximum information. Adding a new combinator should be trivial (one generic
adapter), not an edit to a central enum.

## 1. What we have today, and why the operator is right

`iter.cdz` (monomorphic Int64) and `giter.cdz` (generic) both encode an iterator as a SUM of
step-shapes that a central `next` INTERPRETS at runtime:

```
type Iter = | FromList(...) | MapI((f, src)) | FilterI((p, src)) | TakeI(...) | ...
def next(it) = match it with
  | MapI((f, src)) => (match next(src) with | Some((x, r)) => Some((f(x), MapI((f, r)))) | ... )
  | FilterI((p, src)) => ...
  | ...
```

Consequences the operator is objecting to:
- **Enum dispatch per element.** Every `next` pull runs a `match` over the variant tag, on every
  element, at every layer of the pipeline — a `map().filter()` over N elements does 2N tag
  matches the compiler cannot see through.
- **Central enum = poor extensibility.** A new combinator means a new `Iter` variant AND a new
  `next` arm in the one big match — a central choke point (this is exactly the N×M the original
  design tried to kill at the *traversal* level, reintroduced at the *dispatch* level).
- **No fusion.** The `MapI(f, r)` cons rebuilds a wrapping variant each step; nothing collapses the
  layers into one loop. Intermediate iterator values are materialized between stages.

## 2. The target: adapter-per-combinator, monomorphized, inlined to a fused loop

The Rust model: each combinator is a distinct generic TYPE (`Map<I,F>`, `Filter<I,P>`) that wraps
its upstream iterator `I` and implements `next` by CALLING `I::next` directly (a static, known
call — no tag). The chain's concrete type (`Filter<Map<FromList, F>, P>`) is monomorphized, and
`next` is small enough to inline, so `map().filter().fold()` collapses to one loop.

Cadenza has the two ingredients this needs, **minus a trait system**:
- **Monomorphization:** rcdzc already specializes a generic call per concrete type
  (`emit_call_or_specialize`; a non-recursive generic call monomorphizes away — `lower.rs`).
- **Inliner:** rcdzc has a cost/caller-threshold inliner with `inline_always`/`inline_never`
  markers (`db.rs`, `compile.rs`, `lower.rs`); the self-host has `beta`/`inline`/`cse`/`licm`.
- **MISSING: traits/type-classes** (no `trait Iterator { fn next }`), and **no forall-binders yet**
  (the in-flight v-inference work). So we cannot write `next : forall I: Iterator. I -> ...`
  today. The design must deliver the adapter-per-combinator + fusion WITHOUT a trait.

### 2a. Encoding without traits — the "adapter record carrying its own `next`" (closure-state machine)

Each combinator is a generic function returning a small RECORD that carries (a) its state and (b)
its own `step` CLOSURE `state -> Option (elem, state)`. Because the closure is monomorphized per
chain and the records are non-recursive, the whole pipeline is a concrete nested type the compiler
sees through; marking the `step` closures `inline_always` fuses the layers.

```
// An iterator is a pair (state, step) — step is the per-combinator next-fn, generic over elem.
// (record sugar; today: a tuple. No central enum — each combinator defines its own step.)
type Iter a s = { state: s, step: s -> Option (a, s) }

def from-list-iter(xs) = { state = xs, step = fn(s) => match s with
  | [] => None | [h, .. t] => Some((h, t)) }

// map: GENERIC over the upstream iterator's state type `s` and elem `a`; returns a NEW Iter whose
// step calls the upstream step directly (static call, no tag) then applies f. Trivial to add.
def map(it, f) = { state = it.state, step = fn(s) => match it.step(s) with
  | None => None | Some((x, s2)) => Some((f(x), s2)) }

def filter(it, p) = { state = it.state, step = <recurse skipping non-matching via it.step> }
def fold(it, acc, g) = <drive it.step to exhaustion threading acc>   // the fusing consumer
```

`fold(filter(map(from-list-iter(xs), f), p), 0, +)`: every `step` is a monomorphic closure with a
statically-known callee; `inline_always` on the steps → one loop, no per-element tag match, no
intermediate `Iter` record materialized between stages (they're consumed by the enclosing step).

### 2b. Why records+closures, not a trait

- It's expressible TODAY (records/tuples + closures + monomorphization + inliner all exist).
- Adding a combinator = one generic `def` returning an `Iter` record — no central-enum edit
  (satisfies the extensibility ask).
- Each `step` call is a direct closure application to a known monomorphized closure (not a tag
  dispatch) — the compiler has full information (satisfies the "give the compiler max info" ask).
- Fusion is the existing inliner's job once the steps are marked/known-small (satisfies fusion).

## 3. Compiler support needed (the real dependency list)

**UPDATE 2026-07-18 — dependency picture VALIDATED by spikes (I0 ruled, I1 run).** The abstract deps
below were sharpened into concrete, filed compiler gaps by hand-running the adapter shape. Current
reality, in the order the shape hits them:

0. **[FIXED] Comma-tuple variant-payload construction.** The adapter shape `Mk(s, s -> Option((a,s)))`
   at first could not even be CONSTRUCTED — the comma-tuple `(a,s)` payload was misread as nullary
   (CDZ0201). v-inference fixed it in `bca5da9e0` (a missing `Resolved::Tuple` arm in `type_in_env` +
   `typeval_of`). Construction + typecheck of the `{state, step}` adapter now work (`cdz check` passes).
1. **[BLOCKER, filed → v-memory-safety] Generic-variant stored-closure CALLBACK ownership.** Calling the
   stored `step` closure back (`f(s)`) on a GENERIC variant declines: "borrowing op operand has an
   ownership this backend cannot yet prove" — at a SINGLE instantiation, before any multi-type concern.
   DISCRIMINATOR: a MONOMORPHIC same-shape variant callback RUNS fine; only the generic-variant callback
   declines. This is a Perceus/borrow provability gap over a closure projected from a type-param-arrow
   variant field. Queue repro `mlrepro-generic-variant-stored-closure-callback-borrowing-op-ownership-
   decline.cdz`. **This is now the #1 critical-path gate** — `step(s)` is the adapter's fundamental op;
   the encoding type-checks but cannot RUN until this lowers.
2. **[BLOCKER, known → v-inference] Recursive-generic monomorphization tie (mono ceiling).** A multi-
   element-type `fold`/`map` chain hits the recursive-generic tie the whole giter family shares
   (565f6d184 / e7d78a566 landed pieces; the residual mono ceiling remains). Single-element-type is
   gated by #1 first anyway.
3. **[VALIDATED — LOWER RISK than first thought] Fusion guarantee.** Confirmed by source: the
   `@inline-always`/`@inline-never` SURFACE annotations ALREADY EXIST (`db.rs` `KNOWN_ANNOTATIONS`,
   mapped by `strip_inline_policy`; `@inline-always` on a recursive def is a coded reject). AND rcdzc's
   DEFAULT is to always-inline non-recursive defs (`inline_always` is currently a NO-OP because the
   default already folds — `db.rs:1701`). So the non-recursive combinators (`map`/`filter` return a new
   record) ALREADY inline by default → fusion should be ~free; the recursive DRIVER (`fold-loop`) is the
   one loop we keep. The residual I2 risk narrows to a SINGLE question: does the inliner see THROUGH a
   closure stored in a RECORD/VARIANT FIELD (the `step` field)? Verify via emitted WAT once #1 lands.
4. **Records vs tuples (P2, unchanged).** The `{state,step}` record sugar is equivalent to the 2-field
   variant `Mk(state, step)`; same construction/ownership behavior (the spikes used the variant form).
5. **Forall-binders (P3, unchanged).** NOT required; the trait-less encoding is what the operator ruled
   for. (Forall-binder CORE has since landed in v-syntax, but the design does not depend on it.)

## 4. Increments (each gated, each a landable slice) — STATUS 2026-07-18

- **I0 (design) — ✅ DONE / RULED.** Operator ruled: SHIP the trait-less adapter-record encoding (no
  trait system; ad-hoc polymorphism everywhere; a record with a set of functions + a closure that
  implements the protocol; make the combinators CONST → zero-cost). Recorded as a standing language
  principle (iterators = flagship client). This doc landed as `0d5ac7e25`.
- **I1 (mono spike) — ⏳ PARTIAL / BLOCKED.** Hand-wrote `from-list`/`map`/`fold` in the adapter
  encoding. RESULT: construction + typecheck PASS (dep #0 fixed); RUNNING declines on dep #1 (generic-
  variant closure-callback ownership, filed → v-memory-safety) before the mono-tie (#2) is even reached.
  Re-run after #1 lands.
- **I2 (fusion proof) — BLOCKED on I1.** `@inline-always` already exists + always-inline is the default
  (dep #3), so the marker/inlining is in place; the proof = emitted-WAT check that `map().filter().fold()`
  is ONE loop, no intermediate record, no tag — and specifically that the inliner sees through the
  record/variant-field `step` closure. Coordinate the emit-shape metric with v-wasm-opt. Can't run until
  I1 executes.
- **I3 (port the surface).** Re-express the iter.cdz combinator family in the adapter encoding; keep the
  enum version until I2 proves fusion, then swap. Corpus + @test coverage carried over. (Now lives in the
  extracted `implementation/iterators/` package.)
- **I4 (extensibility demo).** Add a brand-new combinator (e.g. `windows(k)`) touching ONE new `def`, no
  central edit — the operator's "trivial to extend" acceptance criterion.

## 5. Status — operator ruling received; gated on two filed backend/inference gaps

The record-closure-adapter encoding delivers all four asks (generic-over-upstream, monomorphic, fusing
via the always-inline default, one-def extensibility) **without a trait system** — and the operator has
RULED to ship exactly that (§ I0), making it a standing language principle. Spikes have converted the
"abstract deps" into two concrete, filed, owned compiler gaps on the critical path:
- **#1 (v-memory-safety):** generic-variant stored-closure callback ownership — the #1 gate; `step(s)`
  can't run until it lowers.
- **#2 (v-inference):** recursive-generic monomorphization tie (mono ceiling) — the multi-element-type
  gate, shared with the whole giter family.
Fusion (dep #3) is LOWER risk than first feared (always-inline is the default; `@inline-always` exists),
narrowing to "does the inliner fold a record/variant-field closure" — verified via WAT once #1 lands.

**No open operator question remains** (the ship-vs-trait fork was ruled: ship). Next action is purely to
re-run I1 → I2 as v-memory-safety (#1) then v-inference (#2) land their fixes; the design itself is
settled and validated.
