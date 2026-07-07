# Runtime tuple projection works through a `let` — and the direct path is a decline-don't-miscompile violation, not a clean trap

*2026-07-07*

**What happened.** The spike fixed `tuple.N` on a **runtime (`let`-bound) tuple** — the shape a
recursive-descent decoder threads as a `(node, next-index)` pair: `(let ((r (dec b i))) … (tuple.0 r)
… (tuple.1 r))`. Previously `tuple.N` was lowered only for a compile-time-resolvable tuple (an inline
`(tuple …)` or an alias); a `let`-bound tuple *returned from a function* is a genuine value-heap array,
and `tuple.N` on it emitted `unreachable`. The fix: `gen_tuple_access` now emits `arr-get(handle, N)`
for a runtime tuple, unboxing a scalar element to its kind from the operand's static `Shape` (a
`Local` gained a `shape` field so a materialized `let`-bound Heap tuple carries the layout the tag-free
heap does not). Verified: `(let ((r (dec 4))) (+ (tuple.0 r) (tuple.1 r))) → 45`, and the compound
companion `(ev (tuple.0 l))` matches the projected `Node`.

But probing the **`let`-free** path — `tuple.N` applied *directly* to a function's runtime-tuple result
— found it is worse than the spike's handoff recorded. SEED-GAPS calls it "a VALID component that TRAPS
at the renderer (native == wasm agree — a deterministic trap, not a wrong value)". Direct measurement
disagrees: `(def (main) (tuple.0 (dec 4)))` produces an **INVALID component** — `component failed
validation: failed to compile: wasm[0]::function[1]` — and so does the scalar-*consumed* form
`(+ 0 (tuple.0 (dec 4)))`. Only the `let` form compiles: `(let ((l (dec 4))) (tuple.0 l)) → 40` works,
`(tuple.0 (dec 4)) → INVALID`. So the `let` is load-bearing, and its absence is a **decline-don't-
miscompile violation** — an invalid module is strictly worse than a clean decline or a defined trap, and
it is not the "valid-but-traps" state the handoff claims.

**Why.** The fix carried the tuple's `Shape` on the `let`-bound `Local`, so `tuple.N` can recover a
scalar element's kind and `main`'s result kind. Without the `let` there is no `Local` to hang the
`Shape` on: the function result flows straight into `tuple.N`, the operand's layout is not materialized,
and the emitter produces an ill-typed `arr-get`/unbox sequence that fails wasm validation rather than
declining. The boundary is sharpened by the callee's binding form: `tuple.N` applied directly to a
**lambda** result — `(tuple.0 ((fn (x) (tuple x 9)) 7)) → 7` — *works* (a corpus case pins it), because
the lambda is compile-time-reduced so the tuple's shape is statically resolvable; applied directly to a
**named-def** result — `(tuple.0 (dec 4))` — it emits the invalid component, because the named call is
not reduced and there is no `let`-bound `Local` carrying the shape. This is the *same* lambda-vs-named-def
asymmetry that governs HOF inlining (`09-functions.sexp`: a lambda argument inlines into a let-bound HOF
but a named-def HOF declines): the compiler resolves shapes/values through compile-time reduction, and
where reduction does not reach (a named call, no binding) it must decline, not emit invalid code. This is the same lesson the tag-free-runtime rendering work established — the compiler must
*remember* a runtime compound's shape because the heap does not carry it
([[2026-07-05-the-runtime-is-tag-free-rendering-walks-a-static-shape]]) — surfacing on the projection
path: shape recovery is wired to the `let`-binding site, not to the projection operator itself, so a
projection with no binding has no shape to recover and emits invalid code. The durable point beyond the
bug: **"valid component that traps" and "invalid component" are different severities, and a handoff that
records the milder one hides a decline-don't-miscompile violation.** An invalid component is the
category the whole two-compilers gate exists to forbid; it must be measured, not assumed, because the
gate scores it as a *disagreement* (FAIL), not a todo — which is exactly what happened when this shape
was tried as a corpus case, so it cannot be pinned green until the seed either compiles it or declines
it cleanly.

**The requirement it drove.** No corpus case was landed this cycle for the direct-projection shape:
pinning it records the true output (40) as a `(needs …)` case, but the seed emits an invalid component
that the gate scores as a **FAIL** (a miscompile disagreement), not a clean `todo` decline — so adding
it would break the green gate, which the corpus discipline forbids (a decline scores todo; an invalid
component that fails to produce the recorded output is a disagreement). The finding is therefore
recorded here and as **SPEC-BACKLOG item 15**: make the `let`-free `tuple.N` on a runtime-tuple result
either compile (recover the operand's shape at the projection site, not only the binding site) or
**decline cleanly** — never emit an invalid component. Once it declines (or compiles), the corpus case
`(def (main) (tuple.0 (dec 4))) → 40` can be added and will score todo (or pass), restoring the
invariant that every recorded shape is either green or a clean pending decline. The `let`-bound cases
the fix did land are already pinned by a sibling (`05-compound-types.sexp` "a scalar element is
projected from a let-bound runtime tuple" and its compound companion), so the working path is guarded;
this learning guards the boundary the fix left as an invalid-component footgun.
