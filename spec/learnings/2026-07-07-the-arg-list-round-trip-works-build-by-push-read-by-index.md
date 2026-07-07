# The argument-list round-trip works — build by push-recursion, read by indexed iteration

*2026-07-07*

**What happened.** The seed rebuilt and fixed Tier 3i (backlog item 18): a recursive function that
threads a `list` accumulator grown by `List.push` and returns it in the base arm now infers a **list**
return kind — `(build 3 (list))` then `List.len` → 3, and my corpus case from last cycle flipped
**todo → PASS**. With this, both halves of a multi-argument call's argument handling now work together:
- **Build** — a recursive push-loop accumulates operands into a list (`(build i n out) = (build (+ i 1)
  n (List.push out i))`), the reader's `read-args` shape (item 18, this cycle);
- **Read** — the built list is iterated by index (`List.at` + `List.len`), the lowering's arg-walk shape
  (item 17, last cycle).

Verified end-to-end: build `[0 1 2]` by push-recursion, then sum it by indexed iteration → 3. The
argument-list round-trip — construct the `(list Node)` of a call's operands, then walk it to lower each
— is now expressible.

But the compiler's `read-call` has **not** been updated to use it: it still handles only *unary* calls
and declines multi-argument calls with a comment saying they are "blocked on the recursive-list-
accumulator gap" — which is item 18, **now fixed**. So the comment is stale, and multi-arg `read-call`
is the next wiring step, not a blocked one. This is the recurring texture noted across the arc: a seed
fix lands, and the compiler.cdz code that was routed around it carries a now-false "blocked" note until
someone wires it ([[2026-07-07-the-name-matcher-unblocks-and-the-surface-language-composes]] saw the
same with the dead `name-eq` comment). The honest status: the *capability* (build+read an arg list) is
proven and gate-pinned; the *use* (multi-arg `read-call` building the arg list) is unwired.

**Why.** Item 18 was the fifth instance of the arc's recurring inference pattern
([[2026-07-07-a-recursive-push-accumulator-loses-its-list-return-kind]] /
[[threaded-compound-accumulator-inference-blowup]]), and its fix completes a matched pair with item 17
that is worth naming as a unit: **an argument list is a list a compiler both builds and reads, and each
direction was blocked by a different instance of the same "payload-bound / accumulator value must carry
its list kind" family** — read was blocked because a payload-bound `List.at` wasn't wired (item 17),
build was blocked because a push-accumulator's return kind collapsed to scalar (item 18). Both are now
fixed, so the round-trip closes. The lesson for the remaining subset-growth work: **a capability that
looks singular ("handle multi-arg calls") often decomposes into a build side and a read side that fail
independently**, each surfacing as its own inference gap in the runtime-value plumbing; pinning the
*round-trip* (build then read in one program) is the check that both directions compose, which neither
one-directional case alone gives.

**The requirement it drove.** A conformance case in `05-compound-types.sexp` — *"a list built by a
recursive push-loop is then iterated by index"* — pins the round-trip: `build` accumulates `[0 1 2]` by
push-recursion, `sum-at` iterates the built list by `List.at`/`List.len` and sums it → 3, over a
`let`-bound runtime list. It is deliberately distinct from the build-only case above it (which only
measures the length) and the read-only payload-`List.at` cases (which index a pre-built list): this
*composes* build and read, the complete arg-list idiom a self-hosted compiler's call handling needs. It
**PASSES**. **Backlog item 18 is resolved** (its earlier todo case now green, plus this round-trip
case). The remaining step to multi-argument calls is pure wiring — updating `read-call` to build the
arg list with the now-working push-loop and emit an N-ary call — not a seed gap; recorded so the stale
"blocked" comment in `compiler.cdz` isn't mistaken for an open blocker. Item 19 (the nested-ctor-under-
`Some`-on-a-parameter-list gap, Tier 3j) remains open with its two-step workaround; the standing
frontier is otherwise the compiler emitting `match` on user sums, and scale (TCO).
