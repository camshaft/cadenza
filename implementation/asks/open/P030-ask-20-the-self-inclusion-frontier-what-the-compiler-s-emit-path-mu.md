## 20. ⚪ The self-inclusion frontier: what the compiler's emit path must grow to compile its own source

**Finding.** Self-hosting is no longer seed-blocked — every seed gap the spike surfaced is fixed, and
the compiler compiles `module bytes → component` for the multi-def / params / `let` / N-ary-call /
full-operator subset over Int64/Bool. What remains is **subset growth**: the compiler's *own source*
(`compiler.cdz`) uses constructs its *emit path* cannot yet produce. This is a **coverage inventory**,
not a defect — each item is "the compiler doesn't yet emit code for a program that uses X," where the
*seed* compiles X fine (verified: the seed compiles a user-sum `match`; `compiler.cdz`'s emit-side
`Core` has no `KMatch`). **Re-inventoried against the CURRENT source 2026-07-07** (the source has grown
as the loop added features — numbers up from the earlier ~41/~19):
- **`match` on user sums** — THE big one: `compiler.cdz` now has **65 `match` expressions over 11 user
  sum types** (`Node`, `Core`, `Instr`, `Prim`, `Kind`, `Def`, `FList`, `DList`, `Code`, …), the spine of
  every pass. The emit-side `Core` has `KConst`/`KAdd`/…/`KCall`/`KIf`/`KLet`/`KDo` but no user-sum-type
  declaration, construction, or `KMatch`. This is the last major emit construct.
- **Tuples** — **270 `(tuple …)` constructions** (every sum payload / multi-value is a tuple); the emit
  path has no runtime tuple construction/projection. Bound up with the sum-match item (payloads are tuples).
- **`List.*`** — **26 calls** (`List.at` ×17, `List.push` ×9): the source threads `list<Core>`/`list<Node>`
  arg-lists and accumulators; the emit path can't produce runtime list construction/consumption.
- **`Bytes.*` + strings** — **21 `Bytes.*`** (`Bytes.of` ×16, `Bytes.len`/`at`/`concat`) and **144 string
  literals**: the emit path builds the OUTPUT bytes, but the source also *compares*/*slices* bytes and
  carries string literals (op names, diagnostic messages) as runtime values.
- **Effects** — **1 `effect` decl (`Diag`) + a recursive effectful `check` pass** now in the source
  (operator's effects direction); emitting a program that DECLARES/PERFORMS/HANDLES an effect is a whole
  emit sub-system the `Core` has no representation for (and blocked at the compile entry by ask-46).
- **Deep recursion / scale** — the source is pervasively recursive; compiling it walks deep, where the
  bounded wasm stack bites (item 8 / TCO).

**Why it touches the roadmap (not the spec).** This is the concrete distance to *compiler-compiles-
compiler*, framed as what the emit path must cover, not a language question. It is the
already-recorded reframing ([[self-hosting-gate-shifts-from-seed-capability-to-bootstrapping-subset]])
made into a checklist. Not a spec decision; a scope/sequencing view for the operator.

**Status.** ⚪ Roadmap inventory, not a single fix. Priority order for reaching self-inclusion:
(1) emit `match` on user sums + user-sum construction + tuples (the bulk of the source — sum-match and its
tuple payloads are ~half the passes); (2) `List.*` construction/consumption on the emit path; (3) String/Bytes
comparison + slicing ops on the emit path; (4) effect declare/perform/handle emission (now that the source
uses a `Diag` effect — see ask-46 for the compile-entry blocker); (5) TCO for deep tree-walks (item 8).
NOTE — the **scratch-local guard** concern is RESOLVED for the operators that hit it: `compiler.cdz` grew a
3-scratch-local scheme (reserved past the let-locals, `count-checked`/`scratch-count`), so overflow-checked
`+ - *` and the guarded shifts `<< >>` (count-range trap + left-shift overflow guard) are now EMITTED
correctly (verified value+trap vs native), not declined — the earlier "declined until a local-allocating pass
lands" note is superseded. Each remaining item is subset growth the loop will pin as it lands. Learnings:
`spec/learnings/2026-07-07-match-on-user-sums-is-the-last-major-emit-frontier.md`,
`spec/learnings/2026-07-07-a-no-scratch-local-lir-must-decline-ops-that-need-guard-locals.md`.

---

**🔬 EMPIRICALLY CONFIRMED 2026-07-07 (compiler.cdz 19:30, stable seed 18:44) — now that the disagree gate is 0
(commit `1ecdfe6`) and the frontier is the DECLINE pile (commit `55c307e`), I verified ask-20's grep-inventory by
RUNNING the compiler on each construct. Every compound construct DECLINES (bare `unreachable`) where native
compiles it — this IS the 431-decline pile, and it's the self-hosting critical path. CONFIDENCE: HIGH (ran both
compilers + inspected the Core IR directly).**

**Direct confirmation of the "emit-side `Core` has no compound node" claim:** the `Core` type (compiler.cdz L1082)
declares 28 node kinds — ALL scalar/control (`KConst KBoolC KFloat KLocal` / `KAdd…KXor KShl KShr` / `KLt…KNe KNot`
/ `KIf KLet KDo KCall KError`) — and **ZERO** of `KTuple KRecord KMatch KList KSum KString KBytes` (grep: 0 refs
each). So the emit path structurally cannot lower any compound construct.

**Ran-the-compiler confirmation** (native VALID, mine → bare-`unreachable` DECLINE — even when the compound is
built and projected to a SCALAR result, the self-hosting-relevant path):

| construct | native | mine |
|---|---|---|
| `(tuple.0 (tuple 9 8))` → 9 | VALID | DECLINE |
| `(. (record (a 7)(b 2)) a)` → 7 | VALID | DECLINE |
| `(let ((t (tuple 3 4))) (tuple.0 t))` | VALID | DECLINE |
| `(List.len (list 1 2 3))` → 3 | VALID | DECLINE |
| `(match (Some 5) ((Some x) x) …)` → 5 | VALID | DECLINE |
| `(match (S.A unit) ((A u) 1)((B u) 2))` → 1 | VALID | DECLINE |
| tuple/record param projected in a helper | VALID | DECLINE |

**The self-inclusion paradox, made concrete:** compiler.cdz's OWN source uses **75 `match`, 273 `(tuple …)`, 29
`List.at`/`push`** (re-counted) — which the SEED compiles fine (that's why compiler.cdz self-compiles today). But
as a *compiler*, compiler.cdz cannot lower these same constructs in a program it is GIVEN, because its `Core` has
no node for them. So `compiler-compiles-compiler` is blocked precisely on adding
`KMatch`/`KTuple`/`KRecord`/`KList` (+ their construction/projection/consumption lowering) to the emit path.
**Priority unchanged from ask-20's inventory and empirically corroborated: `match` on user sums is THE keystone
(75 uses, the spine of every pass), and tuples (273) are bound up with it (every sum payload is a tuple) — do them
together.** The scalar/control subset (Int64/Bool arith, if/let/do/call, coded diagnostics) is DONE and
gate-green; the compound-value emit subsystem is the whole remaining distance to self-hosting. No new ask — this
is ask-20, now backed by runtime evidence rather than source-grep alone.

---

**🎯 CONCRETE EMIT TARGET for the keystone (KMatch + KSum construct) — extracted from the SEED's own lowering so
the Cadenza-side port has an exact spec, not "implement match." CONFIDENCE: HIGH (disassembled the seed's runtime
sum-match output + read `gen_match_runtime_sum`).** The seed's `gen_match_runtime_sum` (grep the def name —
line drifts; was ~L4529, currently L4702, called from `gen_match_runtime` currently L4650 when the scrutinee kind
is `Heap`) is the forward-port target. The runtime idiom
(disassembled from a NON-foldable sum match — a recursive `(loop n)` returning `(S.A unit)`/`(S.B unit)`, which
defeats the seed's const-folder and forces real heap emission):

```
<scrutinee code>           ; leaves the sum HANDLE on the stack
local.set $h               ; store handle so each arm can re-read it
local.get $h ; call sum-disc ; i32.const <disc A> ; i32.eq
if (result <arm-kind>)               ; arm A (disc A = variant index in declared order)
  local.get $h ; call sum-payload ; local.set $binderA   ; bind payload (whole handle; or arr-get per tuple slot)
  <arm A body>
else
  local.get $h ; call sum-disc ; i32.const <disc B> ; i32.eq
  if (result <arm-kind>)             ; arm B
    local.get $h ; call sum-payload ; local.set $binderB
    <arm B body>
  else unreachable end               ; exhaustive match → innermost else is `unreachable`
end
```

Key facts for the port:
- **Heap imports needed:** `sum-new (disc, payload) → handle` (CONSTRUCTION — `(S.A unit)`), `sum-disc handle → disc`
  (i32 variant index), `sum-payload handle → payload`. compiler.cdz's emit path currently emits **NONE** of these
  (grep of compiler.cdz: 0 `sum-new`/`sum-disc`/`sum-payload` — its heap-import allow-list would need them added,
  the same way `arr-*`/`bytes-*` are wired).
- **Discriminant = variant's index in its sum type's declared order** (matches `gen_runtime_sum`). So `KSum`
  construction is `sum-new(<variant-index>, <payload-handle>)` and `KMatch` dispatches on `sum-disc == index`.
- **Payload binding:** a bare-name binder binds the whole payload handle (Kind::Heap); a `(tuple b0 … bn)` binder
  reads each slot via `arr-get(payload, k)` unboxed per the variant's recorded payload kinds — which is why
  **tuples and sum-match are one unit** (sum payloads ARE tuples; `arr-alloc`/`arr-get`/`arr-set` are the tuple
  primitives, already used by the seed).
- **Exhaustiveness:** a well-typed exhaustive match always selects an arm, so the innermost `else` is
  `unreachable`; a non-exhaustive match is a CDZ0210 rejection (the seed checks bool-match exhaustiveness up front,
  int/sum structurally).
- **The scalar scrutinee case** (`gen_match_runtime` non-Heap path) is a simpler nested-if on the value
  itself — already partly present via compiler.cdz's bool `match`→`if` desugar; the NEW work is the Heap (sum) path.

So the minimal first step toward self-hosting is: add `KSum`+`KMatch` to `Core`, wire `sum-new`/`sum-disc`/
`sum-payload` into the emit path's heap-import list, and lower `KMatch` as the nested-`if`-on-`sum-disc` above
(with `arr-get` payload binding for tuple patterns). That single construct unblocks the 75 `match` sites — the
spine of every compiler.cdz pass — and its tuple payloads. Verify against the seed by diffing the emitted
runtime-sum-match bytes (a recursive-scrutinee probe like `(loop n)` above forces the runtime path, not the folded
one).

---

**🐛 BUG in the in-progress `lce` const-propagation (compiler.cdz ~20:06–20:10): a let-bound literal compound
DECLINES AT THE BINDING SITE, poisoning the whole `let` — even when the binding is UNUSED. The projection-fold
wiring is correct but never gets a chance to fire. CONFIDENCE: HIGH (isolated to the binding site by an
unused-binding probe + source-read).** Progress landed since the keystone note: compound coverage started
const-first (commit `6b9ab7b`) — **inline** const projection now WORKS (`(tuple.0 (tuple 5 6))` → 5,
`(. (record (a 7)) a)` → 7). And the `lce` (literal-compound env) machinery is now fully present: `read-let`
takes `lce` and pushes `(slot voff)` for a literal-compound binding (L1131/1148), `compound-receiver-off` resolves
a bound-name receiver via `lce-at` (L~918), and the `tuple.`/`.` branches call it (L~1010). **But it still
declines** — root-caused:

- `(let ((t (tuple 3 4))) (tuple.0 t))` → DECLINE (native → 3). And critically `(let ((t (tuple 3 4))) 99)` — `t`
  **UNUSED** — **ALSO declines**, while `(let ((x 5)) 99)` (scalar, unused) compiles → 99. So the poison is the
  BINDING SITE, not the projection.
- Root: `read-let` (L1142-1143) emits the `NLet` value as `(read-node b voff …)` — reading the binding's
  `(tuple 3 4)` as a runtime value node. A `(tuple …)` has no `Core` node ⇒ that read DECLINES (→ `unreachable`)
  ⇒ the `NLet`'s value is a declined stub ⇒ the whole `let` traps, regardless of whether `t` is used or whether
  `lce` would fold the projection. The `lce` fold at the USE site is correctly wired but the DECLINED VALUE at the
  BIND site short-circuits first.
- **Fix:** when a binding's value is a literal compound recorded in `lce` (a compile-time-only value with no
  runtime rep on this scalar path), the `NLet` must NOT lower that value as a runtime node. Either (a) elide the
  binding entirely — it exists only to be const-folded at use sites via `lce`, so emit the body directly with the
  slot bound in `lce` only (no `NLet` value emission); or (b) emit a harmless placeholder value (e.g. `KConst 0`)
  for the slot instead of `read-node`-ing the compound, since the real value is served from `lce`. Option (a) is
  cleaner (no dead local). Verify: `(let ((t (tuple 3 4))) (tuple.0 t))` → 3, `(let ((t (tuple 3 4))) 99)` → 99,
  and the nested/composed cases (`(let ((t (tuple (tuple 1 2) 3))) (tuple.0 (tuple.0 t)))` → 1) once the recorded
  `voff` is itself a compound whose projection re-folds.
- ⚠️ Also still declining (same const-first frontier, lower priority): nested/composed INLINE projection where
  the selected element is itself compound — `(tuple.0 (tuple (tuple 1 2) 3))` → DECLINE (native → `(tuple 1 2)`,
  a compound RESULT that can't cross the scalar run boundary anyway) and `(tuple.0 (tuple.0 (tuple (tuple 1 2) 3)))`
  → DECLINE (native → 1, a scalar — SHOULD fold). The composed-to-scalar case is the one worth fixing; it needs
  the inner projection's folded result (itself a literal-compound offset) to be foldable by the outer projection,
  i.e. the fold must recurse through a projected compound element, not just a direct literal head.

This is the const path (no runtime heap) — it does NOT need the `sum-new`/`sum-disc` machinery above; that's the
separate RUNTIME path (recursion/param scrutinee). Both are ask-20 subset growth; the const path is landing now
and this binding-site poison is its current blocker.

---

**✅ BINDING-SITE POISON FIXED + 🐛 NEXT const-frontier blocker isolated: COMPOSED projection (a receiver that is
itself a projection) doesn't fold. CONFIDENCE: HIGH (ran both compilers + source-read of `compound-receiver-off`).**
Re-probed (compiler.cdz 20:10): the binding-site poison above is RESOLVED — `(let ((t (tuple 3 4))) 99)` → 99,
`(let ((t (tuple 3 4))) (tuple.0 t))` → 3, and **let-bound RECORD projection works too** (`(let ((r (record (a 7)
(b 2)))) (. r a))` → 7, `(. r b)` → 2). So inline + let-bound single-level tuple/record const projection is DONE.

**Remaining const-path blocker — composed projection:** a projection whose RECEIVER is itself a projection
declines, when it should fold to a scalar:
| probe | native | mine |
|---|---|---|
| `(tuple.0 (tuple 5 6))` (receiver = literal) | 5 | ✅ 5 |
| `(tuple.1 (tuple (tuple 1 2) 9))` (projects a SCALAR element) | 9 | ✅ 9 |
| `(tuple.0 (tuple.1 (tuple 9 (tuple 1 2))))` (receiver = a projection→compound) | 1 | 🔴 decline |
| `(let ((t (tuple (tuple 1 2) 3))) (tuple.0 (tuple.0 t)))` | 1 | 🔴 decline |

**Root, in `compound-receiver-off` (compiler.cdz L919):** it resolves a projection's receiver from exactly TWO
sources — (a) a literal `(tuple/record …)` array (CBOR major 4 → its own offset), (b) a bound name (major 6 tag →
`lce-at`). It has **no case for a receiver that is ITSELF a projection** (`(tuple.N …)`/`(. …)`). Such a receiver
IS a major-4 array, so `compound-receiver-off` returns its own offset — but the caller then checks
`node-head-is roff "tuple"`, and the node's head is `tuple.0` (not `tuple`), so the literal check fails → decline.
There is no recursion to fold the inner projection to a compound first.

**Fix:** add a third case to `compound-receiver-off` — when the receiver is a `tuple.N`/`.` projection, recursively
resolve ITS receiver's compound offset, project the selected element (tuple element N via `read-child-off`, or
record field via `record-field-value-off`), and return that element's offset if it is itself a literal compound
(or `-1` if scalar/absent). That makes the resolver return "the literal-compound offset this receiver denotes"
transitively, so `(tuple.0 (tuple.1 …))` and `(tuple.0 (tuple.0 t))` fold through the intermediate compound.
Verify: the two 🔴 rows above → 1. (`(tuple.0 (tuple (tuple 1 2) 3))` → `(tuple 1 2)`, a compound RESULT that
can't cross the scalar run boundary, stays a decline — correct; only composed-to-SCALAR is the target.)

This is the last isolated const-path gap before the two big items (const `match`/sum-fold, and `List.*` const);
match-on-const-sum (`(match (Some 7) …)` → 7) and `(List.len (list 1 2 3))` → 3 both still decline and are the
next const increments after composed projection.

---

**🔬 ROOT-CAUSED the const `match`-on-sum gap (compiler.cdz 20:18, after the placeholder-unobservability fix
`f9dec72`): the const-match fold is SCALAR-ONLY; a const SUM scrutinee is never `const?`, so its fold path is
never taken. CONFIDENCE: HIGH (ran both compilers + read `read-match`/`fold-const-match`/`const?`).** Confirmed
the split:
- **Scalar** const-match WORKS: `(match 5 (5 100) (_ 200))` → 100.
- **Sum** const-match DECLINES (native folds all): `(match (Some 7) ((Some x) x) …)` → 7, `(match (None unit) …
  ((None u) 99))` → 99, `(match (S.A unit) ((A u) 10) …)` → 10.

**Mechanism:** `read-match` (L1140) folds a const scrutinee via `fold-const-match` ONLY when `(const? scrut-core)`.
But `const?` (L1319) is true only for `KConst`/`KBoolC` — SCALARS. A sum constructor `(Some 7)`/`(S.A unit)` has
NO `Core` node (Core has no `KSum`, confirmed earlier), so it is never `const?` ⇒ `fold-const-match` is never
invoked for it ⇒ falls through to the bool/arity paths ⇒ decline. And even if reached, `fold-const-match` (L1122)
only matches **int-literal + `_` patterns** against a scalar `sv`; it has no case for **constructor patterns**
(`(Some x)`, `(A u)`).

**Fix — a const SUM-match fold, mirroring the tuple/record projection design (inspect the SURFACE literal, no
`KSum` Core node needed):** when the `match` scrutinee is a literal constructor application `(Ctor payload)` /
`((. Ty Ctor) payload)` (or an `lce`-bound name whose value is one), read the scrutinee's variant name + payload
offset, find the arm whose constructor pattern names that variant, bind the pattern's binder to the payload
(record `slot→payload-offset` in `lce`, or substitute — exactly how a `let` binds), and fold to that arm's body
read under the extended env. `None`/nullary variants bind nothing. No matching arm → decline (safe under-reject).
This is the const-path sibling of the RUNTIME sum-match spec above (`sum-disc`/`sum-payload`); the const path needs
neither — it selects the arm at read time from the known constructor, just as `fold-const-match` selects an
int-arm from a known `sv`. The payload binding rides the SAME `lce` machinery that just landed for
let-bound-compound projection (a `(Some (tuple 1 2))` payload bound to `x`, then `(tuple.0 x)`, composes through
`lce`).

**Priority:** this is THE keystone (75 `match` sites in compiler.cdz's own source). The const path unblocks the
matches whose scrutinee is statically known (a large fraction of a compiler's internal matches are on
just-constructed sums); the runtime path (recursion/param scrutinee, the `sum-disc` spec above) covers the rest.
Do const sum-match next — it reuses the fold+`lce` infrastructure already in place and needs no new heap imports.
(`List.len (list …)` → 3 is the other pending const increment: a `list` literal folded to its length, analogous.)

---

**📊 PRIORITIZATION CORRECTION 2026-07-07 (compiler.cdz 20:28, stable seed refreshed 20:27, gate PASS 134 agree /
0 disagree / 421 declines) — const `List` folding is a GATE win but does LITTLE for self-hosting; the
self-hosting-critical List work is the RUNTIME `List.at`/`List.push` path. CONFIDENCE: HIGH (counted compiler.cdz's
own usage + probed native).** Sizing the two pending const increments by their self-hosting value:

- **const sum-match** — HIGH self-hosting value. compiler.cdz's own source has 75 `match` sites; a large fraction
  scrutinize a just-constructed / statically-known sum, which the const fold covers. Do this next (root cause +
  fix above).
- **const `List`** — LOW self-hosting value, despite being an easy analogous fold. Measured compiler.cdz's own
  usage: `List.at` ×20, `List.push` ×11, `List.len` ×**0**, `(list …)` literal ×29 — but the `List.at`/`push`
  calls are on RUNTIME lists (arg-lists, accumulators threaded through the passes), NOT const literals. So folding
  `List.len (list 1 2 3)` → 3 (native does, 0 heap imports) moves corpus-gate `decline→agree` cases but unblocks
  almost nothing in compiler.cdz's self-compilation. **The self-hosting List blocker is the RUNTIME list path**
  (`List.at`/`List.push` on a value built/threaded at run time — the `vec-*` heap ops), not the const fold.

**Confirmed the `list` gap is recognition-level (distinct from tuple/record):** `read-app` has branches for
`record`/`map` (L984) and `tuple.` (L1027), but **NONE for `list`/`List.*`** — so `(list 1 2 3)` / `List.len` fall
to the unknown-head `"?"` decline (a bare-`unreachable` stub, `Ok (88 bytes)`). tuple/record are recognized-and-
folded; list isn't recognized at all. So const-List needs recognition FIRST, then a fold — more work than
sum-match (which at least reaches `read-match`), for less self-hosting payoff.

**Revised sequencing for the const/compound frontier:** (1) **const sum-match** [keystone, high value, infra
ready]; (2) **composed projection** [`compound-receiver-off` recursion, small, unblocks nested field/tuple access
common in a compiler]; (3) **RUNTIME sum-match + runtime List/tuple** [the `sum-disc`/`vec-*` heap path — the
actual bulk of self-hosting, since compiler.cdz threads runtime lists/sums through every pass]; const-List fold is
a low-priority gate-only nicety, do it opportunistically or skip until the runtime list path lands (which
subsumes it).

---

**📊 SEQUENCING CORRECTION (supersedes "do const sum-match next") — a USAGE CENSUS of compiler.cdz's own 75 match
sites shows const sum-match helps SELF-HOSTING very little; the RUNTIME sum-match + runtime `List.at` path is the
true keystone. CONFIDENCE: HIGH (census + ran the dominant patterns).** I mis-ranked const sum-match last cycle.
Counting how compiler.cdz's own `match` sites scrutinize:
- **50 / 75 match on a bare NAME** (`node`, `xs`, `funcs`, `scrut`, `x`, `d`, `kind` …) — a runtime value (a
  function PARAM or a `let`-bound accumulator threaded through a pass). NOT a static literal ⇒ const-fold can't
  touch these.
- **~20 / 75 match on `(List.at xs i)`** — the #1 computed scrutinee: an `Option` returned by a runtime list
  access. Also runtime.
- Only a small remainder match a just-constructed inline `(Ctor …)` (const-foldable).

So compiler.cdz almost NEVER matches a static const literal — it matches PARAM sums and `List.at` results. **Ran
the dominant patterns; all decline (native compiles all):**
| pattern (self-hosting-representative) | native | mine |
|---|---|---|
| `(match s …)` where `s` is a PARAM sum (50 sites' shape) | 1 | 🔴 decline |
| `(match (List.at xs i) ((Some x) x) …)` (20 sites, #1) | 5 | 🔴 decline |
| runtime `List.at` on a param list | 6 | 🔴 decline |

**Revised keystone = the RUNTIME sum-match + runtime `List.at`/`push` path** (the `sum-disc`/`sum-payload` +
`vec-*`/`arr-get` heap emit — the spec in the "CONCRETE EMIT TARGET" section above). const sum-match is now
DEMOTED: it moves corpus-gate `decline→agree` cases but unblocks almost none of compiler.cdz's self-compilation,
because the compiler's matches are on runtime values. Likewise composed CONST projection is low-value —
compiler.cdz's own source has exactly ONE `(. (. …))` chain and ZERO `tuple.N (tuple.M)` chains (its real
projections are off runtime match-bound payloads / params, not nested const literals).

**Corrected priority for self-hosting (not gate coverage):**
1. **RUNTIME sum-match** — `(match <param/List.at-result> ((Ctor binder) …))` via `sum-disc`+`sum-payload` (needs
   `KMatch`/`KSum` in Core + the 3 sum-* heap imports; the emit shape is spec'd above). This alone unblocks the
   ~70 runtime match sites — the spine of every pass.
2. **RUNTIME `List.at`/`List.push`** — the `vec-*` heap ops for the arg-lists/accumulators the passes thread (the
   `(match (List.at …))` scrutinee needs `List.at` to produce a runtime `Option` first).
3. RUNTIME tuple/record construction+projection (payloads of the above are tuples).
4. const folds (sum/List/composed-projection) — GATE-only niceties, subsumed once the runtime path lands; do
   opportunistically, not on the self-hosting critical path.

Net: the const-first work (which HAS been landing — inline + let-bound tuple/record projection now fold) is real
gate progress but is NOT the self-hosting critical path. To make `compiler-compiles-compiler` move, the RUNTIME
sum/list emit (Core `KMatch`/`KSum` + `sum-*`/`vec-*` imports) is the thing — the const fold was a useful warm-up
that exercised the surface-inspection machinery, but the compiler's own code lives on runtime values.

---

**⚠️ N-PLACE RULE tripped + self-resolved this cycle (compiler.cdz 20:52→21:01) — a new `Core.KCompound` node was
added but SIX Core-walking passes lacked its arm, crashing the compiler; the fix then over-rejected
projection-through-`if`. Both fixed live; gate PASS 136 agree / 0 disagree. Recording the LESSON. CONFIDENCE:
HIGH (caught it mid-flight, censused the walkers).** The compound-value work introduced `Node.NCompound`→
`Core.KCompound` (a check-only leaf, like `KFloat`). But adding a Core node kind must reach EVERY function that
`match`es over `Core` — and six did not:
- **Non-exhaustive `match` = a TRAP (compiler crash), not a decline.** `kind-of`, `well-typed`, `has-kerror`,
  `check-node`, `count-lets`, `count-checked` each matched 26 `Core.K*` variants with NO `KCompound` arm. Any
  program routing a `KCompound` to one of them crashed the compiler (`compile run error … wasm backtrace`, not a
  clean `unreachable` decline). Trigger paths observed: a compound as a CALL ARG (→ `kind-of` via
  `args-have-bool`), a compound LET-value (→ `count-lets`), a compound in a `do`/`if` branch (→ `check-node`/
  `well-typed`). Scalars in the same positions were fine. This is the memory's "3-PLACE RULE" (now N-place): a new
  node kind must be added to emit + infer + shape + EVERY walker, or a missed walker traps.
- **The crash-fix then OVER-REJECTED.** Once the arms were added, projection through an `if`-chosen compound
  (`(. (if c r1 r2) f)`, `(tuple.0 (if c t1 t2))`, a nested `(. (. …) …)` record chain) briefly emitted CDZ0201 —
  native COMPILES these (the receiver is a compound, and member/tuple access on a compound is VALID, not a
  non-record type error). That's a false-reject (worse than a decline). Also fixed live.

**Both resolved by ~21:01 — gate back to PASS (136 agree / 0 disagree / 408 declines).** No action needed; the
durable takeaway for the compiler agent: **when adding a `Core` node kind, grep every `(match … Core.K` site and
add the arm — a missing arm is a hard trap (non-exhaustive match → `unreachable`), and a check pass must treat a
compound receiver/operand as a DECLINE, never a CDZ0201, in positions where native compiles it.** A cheap guard:
a single exhaustive "kind-of-Core" dispatch (or a catch-all arm that declines rather than trapping) would convert
these from crashes to declines. This episode is on the const-compound path (`KCompound`), not the runtime
sum/list keystone — but the SAME rule will bite when `KMatch`/`KSum`/`KList` land: every walker needs their arms
too.

---

**🎯 EMIT TARGET for runtime `List.at` — the OTHER half of the #1 self-hosting pattern (`(match (List.at xs i)
…)`, ~20 sites). Extracted from the seed's `gen_runtime_list_at`. CONFIDENCE: HIGH (read the seed + disasm'd the
full import set).** The runtime sum-match spec above covers CONSUMING an `Option`/sum; but the dominant scrutinee
`(List.at xs i)` must first PRODUCE that `Option`. The seed's `gen_runtime_list_at` (grep the def; currently
codegen.rs L7064) lowers `(List.at v i)` → `Kind::Heap` (an `Option<element>` sum):
1. stash the list handle `v` (i32) and index `i` (i64) to locals;
2. bounds check `in_bounds = (i >= 0) & (i < vec-len(v))` (`vec-len` is i32 → `i64.extend_i32_u` to compare);
3. `if in_bounds { sum-new(Some_disc, vec-get(v, i)) } else { sum-new(None_disc, unit) }` — a runtime `Option`,
   discriminants from `sum_variants["Option"]` (so the SAME `sum-disc`/`sum-payload` a downstream `match` reads).

**Complete heap-import set for the `(match (List.at xs i) …)` chain** (disasm'd from a non-folded probe —
`(def (get xs) (match (List.at xs 0) ((Some x) x) …)) (def (main) (get (list 5 6 7)))` → 5):
`vec-empty`/`vec-push`/`vec-get`/`vec-len`/`vec-update` (the runtime list = 32-way trie), `sum-new`/`sum-disc`/
`sum-payload` (the Option it returns + the match consuming it), `arr-alloc`/`arr-get`/`arr-set`/`arr-len` (tuple
payloads). So the runtime-keystone import block compiler.cdz's emit path must add is `vec-*` + `sum-*` + `arr-*`
together — they co-occur in the single most common self-hosting shape.

**Dependency order for the keystone:** (a) `List.push`/`(list …)` runtime construction (`vec-empty`+`vec-push`) so
a list VALUE exists at run time; (b) `List.at` (`vec-len`/`vec-get` + `sum-new` → runtime `Option`); (c) runtime
sum-`match` (`sum-disc`/`sum-payload` nested-if) to CONSUME it. (a)→(b)→(c) is the exact `(match (List.at (…list
built/threaded…) i) …)` pipeline that 20+ of compiler.cdz's match sites use. Landing all three (plus the Core
`KList`/`KSum`/`KMatch` nodes + their arms in ALL walkers per the N-place rule above) is what makes the runtime
self-hosting path compile.

⚠️ **Stale-ref note:** the seed's codegen.rs line numbers DRIFT every cycle (this ask has already gone
gen_match_runtime L4477→L4650, gen_match_runtime_sum L4529→L4702 in a few cycles). All refs here are by DEF NAME —
grep the function name, don't trust a line number.

---

**✅ COMPOSED PROJECTION LANDED — the const-PROJECTION frontier is now COMPLETE (compiler.cdz 21:05, gate PASS
137 agree). CONFIDENCE: HIGH (ran both compilers over a compositional sweep).** The `compound-receiver-off`
recursion I flagged (a receiver that is itself a projection) is fixed. Verified robust across compositions —
all fold to the correct scalar, matching native, no crash, no over-reject (N-place rule respected):
- deep/composed: `(tuple.0 (tuple.1 (tuple 9 (tuple 1 2))))`→1, 3-deep `(tuple.0 (tuple.0 (tuple.0 …)))`→7;
- let-bound composed: `(let ((t (tuple (tuple 1 2) 3))) (tuple.0 (tuple.0 t)))`→1;
- cross-kind: nested `(. (. (record (r (record (x 9)))) r) x)`→9, `(tuple.0 (. (record (t (tuple 8 9))) t))`→8;
- composes with other forms: projection into arith (`(+ (tuple.0 t) (tuple.1 t))`→7), into an `if` condition
  (→10), a record field as a `match` scrutinee (→100), `let` record + project + arith (→13);
- controls hold: single-level still folds (`(tuple.0 (tuple 5 6))`→5); a compound-RESULT projection
  (`(tuple.0 (tuple (tuple 1 2) 3))`→`(tuple 1 2)`) correctly still declines (can't cross the scalar run boundary).

So the const-first compound work is now: inline + let-bound + composed tuple/record projection ALL fold. **The
only remaining const items are the two documented big ones: const sum-`match` (surface-inspection fold, root-caused
above — `fold-const-match` is scalar-only) and const `List` (gate-only, low self-hosting value). After those, the
frontier is entirely the RUNTIME path (sum-match + `List.at`/`push`, the emit targets spec'd above) — which is
where the actual self-hosting distance is.** No new gap this cycle; recording the closure so the projection
sub-thread isn't re-investigated.
