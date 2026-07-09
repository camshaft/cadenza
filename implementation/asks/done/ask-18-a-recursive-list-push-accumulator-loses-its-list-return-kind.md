## 18. 🟢 A recursive `List.push`-accumulator loses its list return kind — FIXED 2026-07-07

**Finding.** A function that is (a) recursive, (b) threads a `list` accumulator parameter, and (c) grows
it with `List.push` in the recursive call has its RESULT kind inferred as non-list, so `List.len` /
`List.at` on the returned value declines "…of a non-list value". Boundary is exactly the conjunction of
the three — drop any one and it works (non-recursive push, an int accumulator, or a no-push identity
thread all compile). Verified: `(def (build n acc) (if (< n 1) acc (build (- n 1) (List.push acc n))))`
then `List.len` declines.

**Why it touches the seed.** It is now THE blocker for multi-argument user-function calls: the reader
accumulates a call's operands into a `(list Node)` with exactly this push-loop shape (`(read-args … i
out) = (read-args … (+ i 1) (List.push out (read-node …)))`), so the arg list can't be built. Same
inference family as Tier 00 (a base-arm-returned accumulator seeds scalar; `List.push`'s list result
must UPGRADE it, not be collapsed) — a `list`-return instance of the order/position-independent
recursive-result inference the arc keeps hitting. Not a spec gap; seed inference.

**Status.** ⚪ Seed work (SEED-GAPS Tier 3i). Pinned by `05-compound-types.sexp` *"a recursive list
accumulator grown by push and returned in the base arm stays a list"* (`build 3 (list)` → `List.len` =
3), scores **todo** (declines cleanly today). Fix: infer the list/heap return kind for a recursive
function whose accumulator is grown by `List.push`, aligning with the non-recursive `List.push` case and
the push-as-first-argument recursive builder (both already infer list). **Unblocks multi-arg calls.**
Learning: `spec/learnings/2026-07-07-a-recursive-push-accumulator-loses-its-list-return-kind.md`.

**Update (2026-07-07) — 🟢 FIXED.** A recursive push-accumulator now infers a list return (`build 3
(list)` → `List.len` = 3); the todo case flipped **todo → PASS**. Both halves of arg-list handling now
work: build (this, #18) + read (#17). Round-trip verified + pinned: `05-compound-types.sexp` *"a list
built by a recursive push-loop is then iterated by index"* (build `[0 1 2]`, sum by `List.at` → 3).
**Remaining to multi-arg calls is pure WIRING** — `compiler.cdz`'s `read-call` still handles only unary
calls (with a now-stale "blocked" comment); updating it to build the arg list with the push-loop and emit
an N-ary call is not a seed gap. Learning:
`spec/learnings/2026-07-07-the-arg-list-round-trip-works-build-by-push-read-by-index.md`.
