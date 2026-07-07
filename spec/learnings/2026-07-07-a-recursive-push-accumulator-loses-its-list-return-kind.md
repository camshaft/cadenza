# A recursive push-accumulator loses its list return kind — the Tier-00 race again, now blocking the arg-list reader

*2026-07-07*

**What happened.** With payload-bound `List.at` fixed (a call's argument list can now be *read back*),
the spike hit the gap on the *other* side: **building** the argument list. A function that (a) is
recursive, (b) threads a `list` accumulator parameter, and (c) grows it with `List.push` in the
recursive call has its **result kind inferred as a non-list** — so `List.len`/`List.at` on the returned
value declines "…of a non-list value". The boundary is exactly the conjunction of those three: drop any
one and it works. Verified against the seed:
- `(def (build n acc) (if (< n 1) acc (build (- n 1) (List.push acc n))))` then `List.len` → **declines**;
- non-recursive push → 2; recursive with an *int* accumulator → 6; recursive list accumulator with *no*
  push (identity thread) → 0 — all compile.

This is **THE current blocker for multi-argument user-function calls.** The reader accumulates a call's
operands into a `(list Node)` with exactly this shape — `(read-args … i out) = (read-args … (+ i 1)
(List.push out (read-node …)))` — so the arg list cannot be *constructed* until a recursive
push-accumulator infers a list return. (Unary calls, one operand and no list, are unaffected and
already work.)

**Why.** This is the **fifth instance of pattern #1 from the self-hosting arc**
([[2026-07-07-the-self-hosting-arc-what-a-language-hits-growing-to-compile-itself]]): order/position-
independent inference of a recursive function's result. The accumulator `acc` is *returned unchanged*
in the base arm (`(if (< n 1) acc …)`), which seeds its kind at a scalar/default; the recursive call
passes `(List.push acc n)`, whose result kind *should* be `list`/`Heap`, but it is unified against the
scalar seed the wrong way, so the return kind collapses to non-list instead of being upgraded. This is
*exactly* the Tier-00 threaded-compound-accumulator race ([[threaded-compound-accumulator-inference-blowup]]),
which had the same cause (a base-arm-returned accumulator inferred scalar) and the same fix
(back-propagate the heap kind to the accumulator; let the more-defined kind win a constraint race) —
seen before on `Heap` sums, on `Bool` returns, and on compound *shape*, and now on a **`list` return
grown by `List.push`**. The through-line the arc keeps confirming: **a recursive function's result
kind/shape must be inferred independent of which position the growing/self-referential value sits in,
and a producer's concrete result kind (`List.push`→list, a base-arm literal→its kind) must *upgrade* a
scalar-seeded accumulator, never be collapsed by it.** The distinguishing detail from the already-passing
recursive builder (`(build (List.push v i) …)` — push as the *first* argument, which works) is telling:
there the pushed list is the first parameter, forced positionally; here `acc` is returned bare in the
base arm, so the seed is scalar and the upgrade is what's missing — the same asymmetry, position-
dependent, that the fix must erase.

**The requirement it drove.** A conformance case in `05-compound-types.sexp` — *"a recursive list
accumulator grown by push and returned in the base arm stays a list"* (`(def (build n acc) (if (< n 1)
acc (build (- n 1) (List.push acc n))))`, `List.len (build 3 (list))` → 3) — pins the shape. It records
the true oracle (3) and scores **todo** (the seed declines cleanly today, "List.len of a non-list
value"), turning green when a recursive `List.push`-accumulator's return kind is inferred as a list. It
is deliberately distinct from the existing recursive-builder case (push as the *first* argument, which
passes): here the accumulator is returned in the base arm and pushed in a *non-first* argument, the
exact shape that seeds scalar and needs the list upgrade — and the exact shape the reader's
argument-accumulation loop takes. **This unblocks multi-argument calls** once fixed. Recorded as
**SPEC-BACKLOG item 18** (the multi-arg-call arg-list builder blocker), with the secondary gap **3j**
(a nested constructor pattern under `Some` declines when the matched list is a parameter — has a clean
two-step bind-then-match workaround, lower priority) noted alongside it as **item 19**. Both are the
recurring payload-kind / accumulator-kind inference family, extended to the list-building side of the
reader.
