# N-ary calls wired end-to-end — the arg-list round-trip became the feature, as pure wiring

*2026-07-07*

**What happened.** With both halves of the argument-list round-trip fixed in the seed (read via
payload-bound `List.at`, item 17; build via the recursive push-accumulator, item 18), the spike wired
**N-ary user-function calls** through the whole pipeline — exactly the "pure wiring, not a blocker" step
the previous cycle predicted ([[2026-07-07-the-arg-list-round-trip-works-build-by-push-read-by-index]]).
The changes are a clean sweep across the stages:
- `NCall` became `(Tuple Int64 (list Node))` — a function index plus an *argument list* (was unary
  `(Int64 Node)`);
- `read-call` now reads any arity: `read-call-args` is a recursive push-loop over the application's
  operands into a `(list Node)` (the shape item 18 unblocked), nullary reading an empty list;
- `resolve` maps `resolve-args` over the list; `lower` pushes each argument left-to-right (wasm's
  calling convention) via `lower-args` before emitting `call`.

Verified end-to-end from source: `(def (add2 a b) (+ a b)) (add2 20 22)` → 42, and a 3-argument
`(add3 10 20 12)` → 42. The multi-argument-call arc — opened when payload-bound `List.at` first
declined and traced through the recursive-push-accumulator inference gap — is closed.

**Why.** This is the payoff of decomposing a capability into its independently-failing directions and
fixing each as its own inference gap. "Handle multi-argument calls" was never one fix: it was
*read the arg list* (blocked by a payload-bound accessor with no runtime emitter) plus *build the arg
list* (blocked by a push-accumulator's return kind collapsing to scalar) — two instances of the same
runtime-value-kind family, cycles apart, each pinned as its own corpus case that flipped green when the
seed caught up. Once both directions worked, the feature itself was **pure wiring**: no new mechanism,
just threading a `(list Node)` through the pipeline stages that already handle lists and calls. This is
the composition thesis at the *feature* granularity (the same shape the reader showed at the primitive
granularity): a compiler feature is an assembly of capabilities each of which must independently hold,
and once they do, the feature is composition, not invention. The corollary the arc kept demonstrating:
**the honest place to have found the gaps was the round-trip, not the feature** — a one-directional
test (build-only, or read-only) passes while the feature is still impossible, so the round-trip case
(build then read in one program) is what certifies the feature is reachable, and the feature case
(`(add2 20 22)`) is what certifies it was reached.

**The requirement it drove.** A conformance case in `09-functions.sexp` — *"a named multi-argument
function applies to all its arguments at once"* — pins the direct N-ary application `(add2 20 22)` = 42
(and a 3-argument companion `(add3 10 20 12)`), the surface shape the reader's call node and the L-to-R
argument lowering actually handle. It is deliberately distinct from the existing *explicit curried*
case `((add 3) 4)`: by §Functions Are Single-Arity they denote the same program, but the direct
`(f a b)` form is what a program writes and what exercises the argument-list-then-`call` lowering (read
the operands into a list, push left-to-right), rather than the nested single-application form. It
**PASSES**. No new backlog item — the multi-argument-call arc is closed (items 17 and 18 resolved, this
is their composition), and the wiring predicted last cycle is done, not pending. The standing frontier
is now the compiler *emitting* the one major construct its own source still needs the emit path to grow
for — `match` on user sums — plus scale (TCO for deep tree-walks); the multi-def, params, `let`, N-ary
calls, and the full operator/connective surface it needs are all now compilable from bytes.
