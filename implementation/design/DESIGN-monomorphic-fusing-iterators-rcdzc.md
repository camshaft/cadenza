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

1. **Monomorphization through the closure/record chain (P0).** A `map(it, f)` whose `it` is itself
   `filter(map(...))` must fully specialize — the nested record/closure type resolved to a ground
   type per chain. This is the SAME recursive-generic / closure-tie machinery v-inference has been
   landing (565f6d184 element tie, e7d78a566 element-through-generic-callee). The nested-generic
   closure-RESULT over-nest (queue `mlrepro-gmap-closure-returning-generic-call-over-nests-result`)
   is directly on this path — a `map` whose closure returns another generic call is exactly a
   fused stage. **This design is GATED on that residual + the mono ceiling** (giter.cdz already hits
   a per-module instantiation ceiling at ~92 tests — a fused chain multiplies instantiations).
2. **A fusion guarantee, not a hope (P1).** Marking `step` closures `inline_always` + `beta`/`inline`
   must actually collapse the chain. Need: (a) an `@inline`/`inline_always` SURFACE annotation for
   the step closures (today `inline_always` is compiler-internal, `db.rs:4710`); (b) verify the
   inliner sees through a closure stored in a record field (not just a named top-level def call).
   This is the highest-risk unknown — may need an inliner extension for record-field closure calls.
3. **Records (or stay on tuples) (P2).** The sketch uses record sugar; a 2-tuple `(state, step)`
   works today. Records improve readability but aren't load-bearing.
4. **Forall-binders (nice-to-have, P3).** With forall + a trait/row-shape we could write a cleaner
   `Iterator`-like bound. NOT required for the record-closure encoding; it would make the types
   spell nicer. This is the in-flight v-inference work — design does NOT block on it.

## 4. Increments (each gated, each a landable slice)

- **I0 (design, THIS doc).** Operator ruling on the record-closure-adapter approach vs waiting for a
  real trait system. Decision needed: ship the trait-less adapter encoding now, or hold for traits?
- **I1 (spike, blocked on §3.1).** Hand-write `from-list`/`map`/`filter`/`fold` in the adapter
  encoding; confirm a 3-stage chain MONOMORPHIZES at 2 element types (the current giter blocker).
  If it declines CDZ0201 → route to v-inference (it's their tie/ceiling work); do NOT work around.
- **I2 (fusion proof, blocked on §3.2).** Add the `@inline` surface marker + confirm via emitted
  wasm/WAT that a `map().filter().fold()` chain emits ONE loop with no intermediate record alloc and
  no tag match. This is the operator's actual acceptance criterion — pin it with an emit-shape gate
  (coordinate with v-wasm-opt on the size/shape metric).
- **I3 (port the surface).** Re-express the iter.cdz combinator family in the adapter encoding;
  keep the enum version until I2 proves fusion, then swap. Corpus + @test coverage carried over.
- **I4 (extensibility demo).** Add a brand-new combinator (e.g. `windows(k)`) touching ONE new
  `def`, no central edit — the operator's "trivial to extend" acceptance criterion.

## 5. Recommendation / open question for the operator

The record-closure-adapter encoding delivers all four asks (generic-over-upstream, monomorphic,
fusing via inliner, one-def extensibility) **without needing a trait system** — buildable on
today's monomorphization + inliner. The two real risks are (a) it's GATED on the same
recursive-generic/closure-tie + mono-ceiling work v-inference is already driving (a fused chain is
a stress-test of exactly that), and (b) the FUSION guarantee needs an inliner that sees through a
closure in a record field (§3.2) — the one genuinely new compiler capability. 

**OPEN QUESTION for the operator:** ship the trait-less adapter encoding now (pragmatic, buildable,
gated on inference maturing) — OR treat this as motivation to prioritize a real trait/type-class +
forall system first, then build the iterator as its flagship client (cleaner, but a much larger
prerequisite)? My recommendation: **I0→I1 spike now** to de-risk the monomorphization on the
adapter shape (it either works on current inference or gives v-inference a concrete target), and let
the I2 fusion result decide whether the inliner extension is small or argues for the bigger trait
investment.
