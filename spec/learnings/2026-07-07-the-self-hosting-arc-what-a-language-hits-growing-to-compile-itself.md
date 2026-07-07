# The self-hosting arc — what a language hits growing to compile its own compiler, and the four patterns that recurred

*2026-07-07*

**What happened.** Over a dense run of the compiler-in-Cadenza spike, the Cadenza-authored compiler
went from a hand-built backend fragment to reading a whole `(module …)` from its canonical AST bytes
and compiling it to a valid component — `module bytes → component`, end to end, for the
arithmetic/comparison/boolean/`if`/`let`/call/multi-def subset over Int64/Bool. Getting there cleared a
long sequence of seed and spec gaps, each captured in its own dated learning. This entry is the
**synthesis** — the shape of the whole push and the patterns that recurred — so the 25 per-cycle
entries have one place a reader can start from. It drives no new requirement; it is the map of the
territory the individual learnings survey.

The arc, in rough order (each links its learning):
- **The backend exists at all** — compile-time inlining was exponential (Tier 00, an inference gap not
  a recursion guard); the resolved-IR ladder (`resolve → fold → lower → serialize → frame`, emission is
  a serializer, [[2026-07-06-lower-through-a-resolved-ir-so-emission-is-a-serializer]]); the typed
  instruction sum ([[2026-07-05-the-internal-ir-is-a-typed-sum-the-public-ast-stays-homoiconic]]);
  multi-function modules with real calls ([[2026-07-06-the-compiler-emits-a-multi-function-module-with-a-real-call]]);
  type-directed result valtype ([[2026-07-06-result-valtype-is-type-directed-through-an-exhaustive-kind-sum]]).
- **The language had holes a floor-outward corpus missed** — boolean connectives
  ([[2026-07-06-a-language-with-conditionals-still-needs-boolean-connectives]]), the gap-finder thesis
  itself ([[2026-07-06-authoring-the-compiler-surfaces-gaps-a-corpus-grown-from-a-floor-misses]]).
- **The front rung** — nested payload binders
  ([[2026-07-06-the-front-rung-of-a-resolved-ir-compiler-needs-nested-payload-binders]] →
  [[2026-07-07-the-nested-payload-binder-fix-closes-the-front-end]]); runtime strings unblocking
  name dispatch ([[2026-07-07-runtime-strings-unblock-the-name-based-front-rung]]); the recursive-Bool
  name matcher ([[2026-07-07-recursive-bool-return-kind-inference-is-branch-order-dependent]] →
  [[2026-07-07-the-name-matcher-unblocks-and-the-surface-language-composes]]).
- **The reader** — CBOR decode as the input dual of the output spine
  ([[2026-07-07-the-reader-decodes-cbor-as-the-input-dual-of-the-output-spine]]); its three legs
  (dispatch / iterate / atom-decode, [[2026-07-07-the-reader-decode-surface-is-complete-dispatch-iterate-atom]]);
  prelude-index name resolution ([[2026-07-07-the-reader-realizes-the-prelude-index-name-resolution-contract]]);
  name→slot scope resolution with shadowing
  ([[2026-07-07-the-reader-resolves-names-to-local-slots-with-lexical-shadowing]]); wired end-to-end
  ([[2026-07-07-the-reader-is-wired-bytes-to-component-end-to-end]] →
  [[2026-07-07-the-whole-module-reader-is-wired-module-bytes-to-component]]).
- **Runtime-value plumbing** — the built-in Option across a boundary
  ([[2026-07-07-the-reader-gate-is-being-closed-accessor-by-accessor]]), runtime `tuple.N`
  ([[2026-07-07-runtime-tuple-projection-needs-a-let-and-the-direct-path-miscompiles]]), payload-bound
  `List.at` ([[2026-07-07-payload-bound-list-at-fixed-multi-arg-calls-are-representable]]), the
  `Never`-on-heap invariant ([[2026-07-07-the-invalid-component-violation-fixed-and-the-handoff-lags-the-seed]]),
  recursive-shape inference ([[2026-07-07-a-recursive-bytes-fold-infers-its-shape-as-the-direct-result]]).

**Why — the four patterns that recurred.** Beneath the specific gaps, four shapes repeated, and naming
them is the durable payoff:

1. **Order/position-independent inference.** A self-call's kind or shape is a placeholder during a
   function's own inference; a concrete sibling must pin the result regardless of which branch/operand
   the self-call sits in. This *same* race appeared on `Heap` (Tier 00, the compound accumulator), on
   `Bool` (the recursive name matcher), and on compound *shape* (the recursive Bytes fold) — one bug
   family, one fix (let the more-defined/concrete sibling win), three axes.

2. **Payload-bound = runtime, and const-folding hides the runtime gap.** Every accessor must treat a
   value bound from a sum payload as the same runtime handle as a top-level value — but the gap kept
   surfacing there *first* because a top-level literal often const-folds past a missing runtime path.
   The `List.at`-with-no-runtime-emitter miss and the "scale limit" misdiagnosis were both this: a
   const-foldable clean analogue is not evidence the runtime path works
   ([[2026-07-07-the-workaround-was-the-bug-correcting-the-scale-limit-diagnosis]]). **Reduce the
   failing program, don't reason from a clean analogue.**

3. **Input and output are duals over one small vocabulary.** The reader (`bytes → AST`) is built from
   the *same* byte primitives as the emitter (`AST → bytes`): `Bytes.at`/`>>`/`&` compose downward into
   CBOR decode exactly as `Bytes.of`/`concat`/`|` compose upward into LEB128 encode. There is no
   separate "reader runtime" — a self-hosted front end is a composition of small verified byte
   operations, symmetric to the back end.

4. **Write it honestly; the contortion is often the bug.** The two worst-diagnosed gaps were both
   self-inflicted workarounds: an out-of-range-`Bytes` placeholder trap poisoned `resolve`; a
   concat-anchor added to satisfy shape inference. Removing the contortion (a real `KError → unreachable`;
   direct shape inference) both fixed the bug and was the correct design. And the *handoff docs lagged
   the seed* repeatedly — so the loop's rule became **probe the rebuilt seed, never trust an in-flight
   edit or a status note.**

**The requirement it drove.** No new corpus case or requirement — this is a map, not a discovery, and
the individual learnings and their pinned cases stand as the normative record. Its value is
navigational and methodological: a reader wanting to understand *how a language grows to host its own
compiler* starts here and follows the links; and the four patterns are reusable diagnostics for the
work still ahead. The self-hosting **architecture** is complete and gate-witnessed end to end
(`module bytes → component`); what remains is **subset growth** (the compiler emitting its own richer
constructs — `match` on user sums, the last major one) and **scale** (tail-call optimization for deep
tree-walks, [[deep-recursion-traps-at-host-stack-limit]]) before *compiler-compiles-compiler* closes.
The gate for self-hosting is no longer a seed capability but the compiler's accepted subset reaching
self-inclusion ([[2026-07-07-self-hosting-gate-shifts-from-seed-capability-to-bootstrapping-subset]]) —
and the four patterns above are what the remaining subset-growth work will keep encountering.
