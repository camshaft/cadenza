## 57. 🗺️ COVERAGE FRONTIER MAP — the 434 byte-gate declines by feature domain (the post-0-disagree backlog)

**Context.** With the byte gate at 0 disagree (Run 114 milestone — soundness proven), the remaining work is
COVERAGE: turning honest `decline`s into `agree`s. There is no disagree left to chase, so the loop's map shifts
from "which rejections miscompile" to "which features the self-hosted compiler doesn't yet emit." This is the
per-file breakdown of the 434 declines (byte gate, stable 18:44 / compiler.cdz 19:14), sorted by decline count —
the priority order for the coverage phase.

| corpus file | agree | soft | **decline** | note |
|---|---:|---:|---:|---|
| **05-compound-types** | 3 | 0 | **139** | 🔴 THE frontier — runtime records/tuples/lists/maps as RESULTS and operands (the M2 runtime-compound-output gap). ~1/3 of all declines. |
| **02-binding-and-control** | 18 | 10 | **56** | let/match/do control forms the compiler doesn't fully lower |
| **10-bytes** | 1 | 1 | **49** | Bytes operations (slice/concat/at/rope) — almost entirely uncovered |
| **13-strings** | 0 | 0 | **45** | String operations — entirely uncovered (0 agree) |
| **09-functions** | 7 | 6 | **34** | higher-order / closures / function values (ask-21-adjacent) |
| **12-metaprogramming** | 0 | 0 | **26** | quote / AST construction / `Ast.*` (ask-39) |
| **14-effects-and-handlers** | 0 | 1 | **22** | effects at scale (beyond the diagnostics `Diag` path) |
| **06-numeric-model** | 50 | 4 | **17** | mostly COVERED (50 agree); residual = float arith, wide ints |
| **07-type-system** | 15 | 1 | **13** | type-check surface |
| **03-equality-and-observation** | 8 | 1 | **12** | structural equality on compounds |
| **01-literals** | 17 | 1 | **9** | mostly covered; residual = float/large-int literals |
| **11-modules** | 0 | 0 | **7** | module/namespace forms |
| **04-capabilities** | 1 | 0 | **5** | capability routing (CDZ04xx) |
| 15/16/17/18/19 (rows/binary/symbols/units/sets) | 0 | 0 | 0 | all SKIP (feature not in the realized set — not declines) |

**Priority read.** The single highest-leverage coverage target is **runtime compound values** (05-compound-types,
139 declines — and it also underlies chunks of 02/03/09/10/13, since a string/bytes/list operation that returns a
compound hits the same wall). This is the M2 acceptance target ([[m2-acceptance-target-runtime-compound]],
[[runtime-element-compound-output-declines]]): the compiler can PROJECT a runtime compound but not PRODUCE one as
a program result / heap value. Landing runtime-compound *emission* (the value-heap alloc + type-directed renderer
the native seed already has) would convert the largest decline cluster and cascade into the string/bytes/list
files. After that: strings (45) and bytes (49) are big self-contained clusters; functions/closures (34) is the HOF
frontier; metaprogramming (26) is ask-39; effects-at-scale (22).

**Not a bug — a backlog.** Every one of the 434 is an HONEST decline (the compiler refuses cleanly; WRONG=0, 0
traps, byte gate PASS). This map is the coverage roadmap for the self-hosting endgame, not a defect list. Each
cluster that lands moves its declines → agree with no risk to soundness (the gate stays 0-disagree by
construction — a new emit path either matches native or the differential gate catches it).

**Status.** 🗺️ Reference/backlog (loop-produced, Run 115). Not an implementation ask — a prioritized map for the
operator + compiler agent to pick the next coverage push. Related: ask-13 (list/sum pattern surface, part of the
compound frontier), ask-39 (metaprogramming), ask-53/54 (the type-check lattice, now sound), the M2 target.
Learning: `spec/learnings/2026-07-07-past-zero-disagree-the-loop-maps-the-decline-pile-not-the-disagree-frontier.md`.

---

**📈 UPDATE (Run 116) — coverage started moving, compound-first as predicted.** Byte gate 120→**123 agree**,
434→**431 decline** (still 0 disagree, PASS). The +3 are all in **05-compound-types (3→6 agree, 139→136
decline)** — the highest-leverage cluster began falling first, confirming the priority read. NOTE: the
runtime-compound RESULT forms (a `record`/`tuple`/`list` returned from a function — the BULK of the 136) still
decline; the +3 that landed are adjacent compound cases (compile-time-known projections/ops), not the
value-heap-emission sub-cluster yet. So the big cascade (runtime-compound VALUE emission) is still ahead — this
is its leading edge. The live seed rebuilt +17KB (19:31) but native gate unchanged (574) and emission unchanged
vs stable — likely the value-heap/renderer machinery landing native-side before compiler.cdz wires it. Watch for
the record/tuple/list-result forms flipping decline→agree as the cascade lands.

---

**🔎 CLARIFICATION + BOUNDARY ANALYSIS (compiler-side loop, agree 120→123).** The +3 in 05-compound-types this
cycle are NOT compound-value emission — they are three **scalar-receiver ACCESSOR rejections** I added at the
reader (gap-independent, no seed dependency): `(. <scalar> field)` and `(tuple.N <scalar>)` → CDZ0201 (member/tuple
access on a non-record/non-tuple), plus the earlier out-of-range-int / dup-field / malformed-`let`. These use
`node-provably-scalar` = `ck-concrete (ck-of (resolve n) (list))` to catch the case where the receiver is a PROVABLE
scalar; a name/compound receiver still declines. So the "05 cluster falling" is partly rejection-detection, not the
value-emission cascade — the runtime-compound RESULT forms (the bulk) are untouched and still need the seed heap.

**Why the remaining compound-in-scalar-position rejections are NOT gap-independent (I analyzed each; documenting so
the compiler agent knows they belong to the compound-support work, not a reader check):** I probed adding a
`KCompound` check-kind (like `KFloat`) to catch a compound literal in a scalar slot. Native ground-truth shows the
compound cases are almost all **SHAPE-DEPENDENT** — they compile when shapes match, reject when they differ, so a
structural check that can't build/compare shapes would FALSE-REJECT the compiling ones:
- `(= (list 1 2 3) (list 1 2 3))` → **VALID** (native compiles, same shape); `(= (tuple 1 2) (tuple 1 2 3))` → CDZ0201
- `(if true (list 1) (list 2))` → **VALID** (both branches same-shape); `(if true (tuple 1 2) 20)` → CDZ0201
- Only UNCONDITIONAL rejections (compound always wrong regardless of shape): compound in an `if`-CONDITION, a `not`
  operand, or an ARITH operand. But the corpus has just **ONE** such case (`(if (tuple 1 2) 10 20)`), and catching it
  safely requires a `KCompound` that is provably-not-bool/not-i64 YET never a provable-MISMATCH (or it false-rejects
  the same-shape equality/branch cases via the shared `ck-provably-mismatch`). A full new Core node (12 exhaustive
  matches) + mismatch-leak risk for 1 case is low-leverage and gap-adjacent — DEFERRED to the compound frontier,
  where the seed's real type-checker catches ALL of these (incl. the shape-dependent ones) uniformly and correctly.

**Takeaway:** the gap-independent reader-level REJECTION seam is now EXHAUSTED (out-of-range int, dup field/key,
malformed `let`, non-exhaustive bool match, int/float, scalar-receiver `.`/`tuple.N`). Every remaining decline is
genuine COVERAGE needing seed support: runtime-compound VALUE emission (the 136 in 05 + cascade), strings (45),
bytes (49), closures (34), metaprogramming (26), effects-at-scale (22). No more decline→agree is reachable from
compiler.cdz alone until the seed grows compound/string/heap emission.

---

**📈 UPDATE (Run 118) — the compound frontier has TWO TIERS; track them separately.** Byte gate 123→**126 agree**
(0 disagree, PASS); 05-compound-types 6→**9 agree, 132 decline**. But the +3 are all CONST-foldable compound
projections (`(tuple.0 (tuple 7 9))` → folds to `i64.const 7`, no runtime compound built). Verified the
runtime-heap tier is UNTOUCHED: `(tuple.0 (mk 5))` (project off a runtime-built tuple) and `(f 3)`→`(tuple 3 1)`
(return a runtime tuple) BOTH still decline. So the 132 remaining in 05 split into:
- **const-foldable tier** (cheap — projections/ops on literal compounds fold to scalars, no value heap): filling
  in now, inflates the agree count, says NOTHING about the runtime capability.
- **runtime-heap tier** (the M2 cascade — value-heap alloc + type-directed renderer): the BULK, and the shared
  capability that strings/bytes/list RESULT cases also ride. Still at zero.
**Priority read unchanged but sharpened:** the leverage is entirely in the runtime-heap tier. Watch a
CALL-PRODUCED compound (not a literal) flip decline→agree as the signal the real M2 machinery landed — a literal
compound folding is not that signal. Discriminator probe pair: `(tuple.0 (tuple 7 9))` [const, agrees now] vs
`(tuple.0 (mk 5))` [runtime, still declines]. Learning: `compound-coverage-lands-const-first-because-folding-
needs-no-runtime-heap`.

---

**📈 UPDATE (Run 120) — const-compound tier still advancing (agree 126→129); runtime-heap tier still zero.** Byte
gate 129 agree / 0 disagree / 26 soft / 424 decline (PASS). The +3 are LET-bound compound projections now
const-folding (`(let ((p (record (x 1)(y 2)))) (. p y))` → 2) — still the const-foldable tier (via a
literal-compound env `lce` + constant propagation). ⚠ SAFETY verified: the fold leaves a placeholder `(NInt 0)`
in the dead slot, and a BARE use of a compound-let binding DECLINES (never leaks 0 as a value) — full sweep 0
disagree confirms the placeholder is unobservable. The runtime-heap tier (call-produced compounds — `(tuple.0
(mk 5))`, `(f 3)`→`(tuple 3 1)`) is STILL at zero. So the const tier is now well-covered (direct + let-bound
projections); the next real leverage remains runtime-compound VALUE emission (M2). Learning:
`a-const-fold-placeholder-must-be-unobservable-decline-every-use-that-would-read-it`.

---

**📈 UPDATE (Run 121) — const tier ~exhausting, runtime-heap cliff untouched. Trajectory:**

| cycle | byte-gate agree | 05-compound agree | what landed (all CONST tier) |
|---|---|---|---|
| 115 (map baseline) | 120 | 3 | — |
| 116 | 123 | 6 | adjacent compound cases |
| 118 | 126 | 9 | direct-literal compound projection fold |
| 120 | 129 | (11) | let-bound compound projection fold (+ placeholder safety) |
| 121 | **132** | **12** | const-scalar literal-pattern MATCH fold |

Steady ~+3 agree/cycle, ALL in the const-foldable tier (direct/let-bound projections, const-match), each verified
decline-don't-miscompile at the runtime boundary (0 disagree throughout). BUT **05-compound still has 130
declines** — the runtime-heap tier (call-produced compounds `(tuple.0 (mk 5))`, `(f 3)`→`(tuple 3 1)`, HOF) is
STILL at zero after 6 cycles. The const tier has headroom left (const-match, const-equality, more), but the
DOMINANT remaining work is runtime-compound VALUE emission (M2) — the cliff nobody's climbed. The coverage is
advancing along the path of least resistance; the leverage cliff remains. Watch a CALL-PRODUCED compound flip
decline→agree as the M2 signal. (Native corpus also still growing: todo 5→6 this cycle — new cases native can't
yet compile.)

**Run 122 tick:** byte gate **134 agree** / 0 disagree / 26 soft / 421 decline (PASS). +2 = type-annotation
`(: expr Type)` transparent-erase (`(: 42 Int64)`→42) + CDZ0203 contradiction rejection. Still const/scalar tier
(reuses ck-of + the ask-56 "every code needs a code-string case" discipline; unprovable-param annotation erases
conservatively, no false CDZ0203 — verified). M2 runtime-heap STILL zero (rt tuple result declines). Native seed
refreshed → gate 574→**577** (todo 6→4: native gained ~3 cases, HOF/effects). const-tier trajectory continues
~+2-3/cycle; M2 cliff unclimbed.

**Run 123 tick — the SOFT door opened (track agree+soft, not just agree).** Byte gate 134 agree / 0 disagree /
**37 soft** (was 26) / **410 decline** (was 421). Agree HELD but ~11 runtime-SCALAR function/binding cases (multi-arg
fns `(add3 a b c)`, let-in-function) moved decline→SOFT — the compiler now EMITS value-correct code for them,
byte-differing from native (the runtime-scalar emit path maturing). This is real coverage progress agree-counting
misses: coverage = agree+soft = **171** cases the compiler runs correctly (agree=134 byte-identical subset).
decline=410 is the true not-yet-covered. M2 runtime-COMPOUND + HOF still decline (soft gain is scalar fns, not
compound/closures). Follow-on: byte-fidelity tuning converts soft→agree. Watch a CLASS moving decline→soft as the
signal a new emit path came online. Learning: `coverage-advances-through-two-doors-decline-to-agree-and-decline-to-soft`.

**Run 128 tick:** byte gate **139 agree** / 0 disagree / 39 soft / 404 decline (PASS). +1 = `do`-block leading
value-def (`(do (def x 5) (+ x 1))` → soft, runtime-scalar emit; a FUNCTION-def in do declines — boundary holds).
Coverage = agree+soft = 178. Still incremental scalar/const/do-block; M2 runtime-compound + ask-59 Bool-params +
ask-58 + HOF all still deferred subsystems (unchanged). The gap-independent scalar slices are nearly exhausted —
the remaining declines (404) are dominated by the substantial subsystems.

**Run 132 — ask-60 (M2 HEAP TYPES) STARTED (operator-directed "next big thing").** compiler.cdz landed step 1:
the `int-to-decimal` value renderer leaf (verified standalone, UNUSED so far → gate 139/0 unchanged, 0 risk). The
resource-with-display ABI is decoded (compound-returning program = make/display/cabi_realloc/memory exports +
heap-runtime import; const `(tuple 1 2)`=673B fixed-blob + length-parameterized display splice). No coverage
movement yet (both const `(tuple 1 2)` and runtime `(f 3)`→tuple still decline) — this is the START of the M2
climb, not the landing. ✅ CORPUS IS READY: the M2 compound-RESULT targets are ALL already pinned in
05-compound-types (const tuple result :363, runtime-element tuple :352, 3-elem :389, bool-elem :398, nested :408,
record :437, list :458, constructor :518) — they'll flip decline→agree/soft as ask-60 wires the renderer. Watch
the CONST compound-result (`(main)=(tuple 1 2)`, :363) flip FIRST (fixed-blob, simplest), then runtime-element.
