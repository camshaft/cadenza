## 53. 🔴 (compiler.cdz — MINE to fix, not a seed gap) The diagnostics `check` pass conflates DECLINES (unsupported constructs) with type REJECTIONS — false-rejects float/string/unit/list

**Context.** The full effect-based diagnostics pipeline now WORKS end-to-end (seed side complete: ask-41 artifact
ABI + ask-45/46/49 recursive-effectful handle at both entries + ask-51 ABI-detection-through-handle + runtime
record field access). Wiring compiler.cdz's `compile` to the `Diag`-handler `compile-output` record and driving
it proved the MECHANISM: well-typed `(+ 3 5)` → `Ok (89 bytes)` component; ill-typed `(+ 1 true)` / `(if true 1
false)` / `(+ 1)` / `(< 1 true)` → `Diagnostics: [("CDZ0201", …)]`. Diagnostics-via-effects is real.

**The bug (compiler.cdz, not the seed).** But `component-check` then showed 441 disagree (was ~152): the coarse
`check-node` pass FALSE-REJECTS programs native COMPILES. Root cause: `check-node`'s `((Core.KError _) (emit-diag))`
arm emits a CDZ0201 for EVERY `KError` — but `KError` has TWO sources that resolve identically:
- **genuine rejection** — a malformed-arity form (`(+ 1)`), an unknown head → native rejects CDZ0201 ✓ (should emit)
- **unsupported construct / decline** — a float literal, a string literal, `unit`, a runtime `list` → native
  COMPILES these; compiler.cdz just lacks the feature (an honest DECLINE). Emitting CDZ0201 here is a FALSE
  rejection (`a floating-point literal` → native ok(96 bytes), mine → `diagnostics["CDZ0201"]`; a deep
  runtime-list let-chain → mine emits 13 spurious CDZ0201s).

So the check pass treats "I can't type this because it's a construct I don't support" the same as "this is a
type error" — but only the latter is a diagnostic; the former is a decline (the emit path already lowers it to
`unreachable`). Since resolve produces ONE `Core.KError` for both, `check-node` can't tell them apart.

**Fix (compiler.cdz — this is my work, tracked here so the state is clear):** distinguish the two KError
sources. Options: (a) split `KError` into `KReject` (malformed/type error → CDZ0201) vs `KDecline` (unsupported
construct → no diagnostic, lowers to `unreachable`); (b) tag `KError` with a boolean/enum "is-diagnostic". Then
`check-node` emits ONLY for `KReject` (and the genuine i64/Bool mismatches it already detects via
`check-arith`/`check-cmp`/`is-bool`), and treats `KDecline` as silent. The malformed-arity path (`read-app`'s
`(malformed)`) and the type-mismatch path (`typecheck`) produce `KReject`; the unsupported-construct paths
(float/string/unit/list/unknown-head in the reader + resolve) produce `KDecline`.

**Until fixed:** `compile` stays bare-`Bytes` (gate green, 27 agree / 0 hard / 0 error). The `Diag` decl +
`check-*` pass + `mk-diagnostic`/`codes->diagnostics` + `compile-artifact` (the artifact-ABI `Diag`-handler
`compile-output` body) are all IN the file, dormant and proven-working via `compile-run`; activation is swapping
`compile`'s body to `(match (List.at inputs 0) ((Some a) (compile-artifact (. a bytes))) …)` once the
decline-vs-reject split lands so it doesn't false-reject.

**Acceptance signal.** With the KError split, `compile` = the artifact-ABI `Diag` body, and `component-check`
scores the ~30 ill-typed cases `agree` (CDZ0201 matches native) WITHOUT regressing the success cases (float /
string / list / unit stay `decline`, as they are today — native compiles them, mine honestly declines, neither a
false CDZ0201 nor a wrong value). Net: disagree DROPS (rejections → agree), declines HOLD (unsupported stay
declines).

**UPDATE 2026-07-07 — HALF FIXED (KError split landed); the deeper half is the coarse kind lattice.** Split
`KError` → `(KError 0)` DECLINE (unsupported construct — silent) vs `(KError 1)` REJECT (type/malformed error —
CDZ0201), threaded through resolve (`PUnknown`→0 / new `PReject`→1), the reader (`malformed`=`"!"`→reject,
new `declined`=`"?"`→decline; user-sum-match → decline), `typecheck` (→reject), and `check-node`'s KError arm
(emit ONLY on k=1). Kept as a permanent IR improvement (behavior-preserving with bare-Bytes; gate green 27
agree/0 hard/0 error). Re-probed the activated diagnostics `compile`: component-check dropped **441 → 260**
disagree — the KError-arm false-flags are GONE (float/string/unknown-head now silent declines, verified `Ok
stub` not `CDZ0201`). **But 260 remain, and ~half are a SECOND false-flag source I underestimated:**
`check-arith`/`check-cmp`/`is-bool` emit CDZ0201 whenever an operand's `kind-of` isn't i64/Bool — and a COMPOUND
operand (list/tuple/Option/unit) has no i64/Bool kind, so a WELL-TYPED compound expression false-rejects:
`(do (list 1 2 3) 7)`, `expect` on a runtime Option, a deep runtime-list let-chain (→ 12 spurious CDZ0201s), unit.
**Root: the coarse `Kind = Ki64 | KBool` lattice CANNOT represent a compound**, so the check can't distinguish
"Bool where i64 needed" (a genuine reject) from "list where i64 needed" (a decline — native compiles it). The
check must be CONSERVATIVE: emit ONLY when it can POSITIVELY prove both operands are KNOWN scalars that mismatch;
a compound/unknown operand is a decline. That needs a richer kind (`Ki64 | KBool | KCompound | KUnknown`) or an
operand "is-definitely-scalar-mismatched" predicate. This is the real remaining design work — larger than the
KError arm, and the reason `compile` stays bare-Bytes. (The other ~half of the 260 are pre-existing under-declines
native rejects that this compiler doesn't detect — out-of-range int literal, unbound name CDZ0101, int-vs-float
CDZ0301, literal-pattern-type-mismatch, non-exhaustive-match CDZ0210 — unchanged by this work.)

**Status.** 🔴 compiler.cdz design (mine). KError decline/reject split: DONE. Remaining: make the type-check
CONSERVATIVE about compound/unknown operands (needs a richer kind lattice than the coarse i64/Bool) so it emits
CDZ0201 only for provable scalar mismatches, never for a well-typed compound expression. The seed side of
diagnostics-via-effects is DONE and proven; this compiler-side check precision is the last thing before `compile`
can return the diagnostics record gate-green. Related: ask-30 (the rejections this surfaces), ask-41/45/46/49/51
(the complete seed pipeline this rides), ask-26/33 (decline-vs-reject).

---

**🔎 LOOP 2nd-probe — MECHANISM CONFIRMED + a correction to the "compound operand → decline" framing that
changes the conservative-check design. CONFIDENCE: HIGH (native ground-truth + source read).**

Independently reproduced the residual on the LIVE activated `compile` (artifact ABI, stable 17:00 & fresh 17:05,
before the agent reverted to bare-Bytes): `(list 1 2 3)` → `Diagnostics:[CDZ0201]` while native compiles it —
the compound false-flag, live.

**Mechanism, pinned in source:** the `Core` IR has **NO compound-producing node** — no `KList`/`KRecord`/`KStr`/
`KTuple`/`KFloat`/`KUnit`; every compound/unsupported construct lowers to **`Core.KError`**. And
`kind-of (Core.KError _)` returns **`Ki64`** (`compiler.cdz:1625`). So the 2-variant lattice doesn't just *lack* a
compound kind — it actively **mis-labels every compound as `Ki64`**. That means a compound operand silently
*passes* an `is-i64` check (wrong-accept) and *fails* an `is-bool`/branch-`kind-eq` check as "Ki64≠KBool"
(wrong-reject). The `KError→Ki64` line is the specific thing to change alongside adding the kind.

**⚠️ Correction to the framing (the part that affects the fix): a compound where a scalar is STRUCTURALLY
REQUIRED is a REJECT, not a decline — native REJECTS it.** The UPDATE above reads "list where i64 needed →
native compiles it → decline," but that is only true in value-DISCARD position. Native ground-truth (via `emit`
on the reference seed, stable 17:00):

| program | native verdict | ∴ compiler.cdz should |
|---|---|---|
| `(do (list 1 2 3) 7)` | ✅ **compiles** (list is discarded) | be SILENT (decline) |
| `(if (list 1) 1 2)` — compound CONDITION | 🔴 rejects "condition is not a Bool" | emit CDZ0201 |
| `(if true 1 (list 2))` — branches int vs list | 🔴 rejects "branches have different types" | emit CDZ0201 |
| `(if true 1 "x")` — branches int vs string | 🔴 rejects "branches have different types" | emit CDZ0201 |
| `(+ (list 1) 2)` — compound arith operand | 🔴 rejects "operation on mismatched types" | emit CDZ0201 |
| `(not (list 1))` — compound `not` operand | 🔴 rejects "negation operand is not a Bool" | emit CDZ0201 |

So the conservative rule is NOT "compound operand ⇒ always decline." It is: **an operand/branch that is a
compound where the form requires a scalar (arith operand, comparison operand, `if`/`not` condition, disagreeing
`if` branches) is a genuine REJECT (CDZ0201) — matching native**; a compound is a DECLINE only where the language
accepts ANY value (a discarded `do` non-tail form; a `let`-binding whose value is compound but unused in a scalar
position; the whole-program result being a compound the emit path lowers to a stub). The distinguishing axis is
**"is a scalar structurally required at this position?"**, not "is the operand a compound?". A `KUnknown`/
`KCompound` kind lets the check answer this: at a scalar-required position, `KCompound`/`KBool`-vs-`Ki64` are BOTH
mismatches to emit on; at a value-any position, kind is not consulted.

**Practical suggestion (CONFIDENCE: MEDIUM on exact shape, HIGH on direction):** add `KCompound` (return it from
`kind-of` for `Core.KError` AND for any future compound Core node), and change the check so `check-arith`/
`check-cmp` emit when EITHER operand's kind ≠ the required scalar kind (i64 for arith, matching for cmp) —
`KCompound` at those positions is a legitimate emit, because native rejects it too (table above). The ONLY place
`KCompound` must stay silent is a value-discard position, which the walk already doesn't scalar-check (a `do`
non-tail form isn't an arith/cmp/if-cond operand). If the live `(list 1 2 3)`→CDZ0201 came from the WHOLE-PROGRAM
result being a bare list (not an operand), that path should decline (it lowers to a stub today) — worth checking
whether `check-node` is invoked on the top-level result node and treating a bare-compound tail as decline.

**Net:** the fix is a 3rd kind + `KError→KCompound` in `kind-of`, and the emit rule keyed on scalar-required
position — but the emit set is LARGER than "only positively-proven scalar mismatch," because a compound in a
scalar slot IS a native rejection. Verifying each of the 6 rows above against the reactivated check would confirm
the boundary before flipping `compile` back on.

---

**🔎 LOOP 3rd-probe (2026-07-07 ~17:26) — RAN the 6-row table against the NOW-LIVE artifact-ABI check
(snapshot of compiler.cdz 17:25:30, seed stable 17:00). The KError-split half is holding; the residual is
EXACTLY the `kind-of KError = Ki64` conflation, confirmed empirically. CONFIDENCE: HIGH.** The decline class
(float / string / unit / `(list 1 2 3)`) is now correctly SILENT (`Ok`) — the two-cycles-ago false-reject on
`(list 1 2 3)` is GONE. And the CONDITION/NOT compound cases correctly emit. But 3 rows still UNDER-REJECT
(compiler accepts, native rejects), and they partition cleanly by which check helper fires:

| position | probe | mine | native | verdict |
|---|---|---|---|---|
| discard | `(do (list 1 2 3) 7)` | ok | ok | ✅ correct (silent) |
| `if` **cond** | `(if (list 1) 1 2)` | CDZ0201 | reject | ✅ correct (via `is-bool`) |
| `not` operand | `(not (list 1))` | CDZ0201 | reject | ✅ correct (via `is-bool`) |
| **arith operand** | `(+ (list 1) 2)`, `(+ "x" 2)`, `(+ unit 2)` | **ok** | reject | ⚠ **UNDER-REJECT** |
| **cmp operand** | `(< (list 1) 2)`, `(< "x" 2)` | **ok** | reject | ⚠ **UNDER-REJECT** |
| **`if` branch mismatch** | `(if true 1 (list 2))`, `(if true 1 "x")`, `(if true 1 unit)` | **ok** | reject | ⚠ **UNDER-REJECT** |

**Root, now empirically confirmed:** `kind-of (Core.KError _)` returns `Ki64` (`compiler.cdz:~1625`), and every
compound lowers to `KError`. So a compound operand has kind `Ki64`, which means:
- `is-i64 (compound) = TRUE` → `check-arith` sees two "i64"s in `(+ (list 1) 2)` → no mismatch → **misses**.
- branch `kind-eq (Ki64) (Ki64) = TRUE` → `(if true 1 (list 2))` sees "matching" branches → **misses**.
- BUT `is-bool (compound) = FALSE` → `(if (list 1) …)` / `(not (list 1))` mismatch → **catches** (why cond/not
  already work).

This is a POSITIVE confirmation of the `KCompound` prescription above: the exact three helpers that consult
`is-i64`/branch-`kind-eq` (rather than `is-bool`) are the ones that leak, and a compound is mis-labeled `Ki64`
precisely there. Generalized across list/string/unit operands (table rows are representative — all three compound
kinds under-reject identically), and the Bool controls (`(+ 1 true)`, `(if true 1 false)`, `(< 1 true)`) all
correctly emit CDZ0201 — so the leak is specific to the UNREPRESENTABLE (compound) kind, not a check-logic bug.

**Fix (unchanged direction, now with the exact acceptance set): add `KCompound`, return it from `kind-of` for
`Core.KError`, and have `check-arith` emit when either operand is not `Ki64`, `check-cmp` emit when the two
operand kinds differ OR either is `KCompound`, and the `if`-branch check emit when the branch kinds differ under
the 3-kind `kind-eq` (`KCompound ≠ Ki64`).** That flips exactly the 3 UNDER-REJECT rows to CDZ0201 while leaving
the discard row silent (it's never a scalar-required position). Acceptance: re-run this 6-row table + the
list/string/unit generalizations — all scalar-position compounds → CDZ0201, discard → Ok, decline class (bare
float/string/list/unit as the whole result) → Ok. ⚠ NOTE: the whole-program-result-is-a-bare-compound case
(`(def (main) (list 1 2 3))`) must STAY silent (native compiles it) — so the `KCompound` emit rule must fire only
at arith/cmp/cond/branch POSITIONS, never on a bare tail value; the current check already gets this right (the
decline class is silent), so the change is additive to the scalar-position helpers only.

**Process note:** compiler.cdz was churning under heavy contention this cycle (two `cadenza-seed` at 100% CPU, a
gate running); I probed against a `cp` snapshot to keep the matrix self-consistent. The earlier paren breakage
(last cycle's line-791 report) is FIXED — read-app closes correctly at L791, `compile-artifact` closes at L2435;
compiler.cdz parses + self-compiles clean (`VALID`, `Ok 89 bytes`).

---

## ✅ RESOLVED 2026-07-07 (compiler.cdz — MINE) — the conservative-lattice fix landed; diagnostics `compile` now GATE-SAFE

Added a THREE-VALUED check kind `CKind = (CKi64 | CKBool | CKUnk)`, distinct from the two-valued codegen `Kind`.
The bug was that the check reused `kind-of`, which DEFAULTS a parameter / call / KError to `Ki64` (so the framing
can pick a valtype) — and the check read that DEFAULT as a positive FACT, so a Bool PARAMETER used as an `if`
condition or a recursive Bool predicate false-rejected (native infers Bool → COMPILES). New `ck-of` returns a
concrete kind ONLY where PROVABLE from the node itself (literal / arith / cmp result); a parameter (`KLocal`), a
call (`KCall`), a compound/unsupported (`KError`), and a disagreeing `if` are all `CKUnk`. The check emits ONLY on
a PROVABLE mismatch — `ck-provably-not-i64` (concrete Bool operand of arith), `ck-provably-not-bool` (concrete i64
`if`-cond / `not` operand), `ck-provably-mismatch` (concrete i64-vs-Bool compare / `if`-branches) — so an UNKNOWN
never fires. Applied to BOTH twins in lockstep: `well-typed?`/`typecheck` (which replaces the body with `(KError
1)` → trap+CDZ0201) and `check-node`/`check-arith`/`check-cmp` (the Diag-effect emitter). Retired the now-dead
`is-i64`/`is-bool`/`kind-eq`.

**Gate deltas (activated artifact-ABI diagnostics `compile`, seed stable):**
- Value-harness: **0 hard / 0 error** (27 agree / 5 soft) — NO miscompile; the correctness gate holds.
- component-check: **79 → 95 agree**, false-rejections (native=ok / mine=diagnostics) **9 → 0**, disagree 104 → 96.
- The 9 gone false-rejects were ALL the Bool-in-scalar-position family (`(if b …)` on a Bool param, conjunction/
  disjunction, recursive Bool predicates) — now silent (agree/decline), matching native's `ok`.
- All 96 remaining disagreements are ONE direction — native=rejected / mine=ok — the pre-existing **ask-30
  under-reject frontier** (out-of-range literal, unbound name CDZ0101, non-exhaustive CDZ0210, pattern arity,
  effect routing CDZ0401/0404, int-vs-float CDZ0301, runtime float eq). None new; none miscompiles.

**⚠️ Design correction to this ask's OWN 3rd-probe prescription (`KError → KCompound`, emit on a compound arith
operand): that is UNSAFE and I deliberately did NOT do it.** `KError` conflates TWO sources — a genuine compound
(`(list 1)` in a scalar slot, which native rejects) AND an i64-valued **decline** (`(+ (Option.expect (List.at xs
0) m) 2)` — native COMPILES to i64 arithmetic; mine declines the `expect` to `KError`). Treating `KError` as
provably-compound would FALSE-REJECT the decline case. Since the coarse surface can't tell them apart, `KError →
CKUnk` (under-reject = a safe decline) is the ONLY sound mapping — decline-don't-miscompile. The 0-false-reject
result confirms it. (The 3rd-probe's 3 "under-reject" rows for `(+ (list 1) 2)` etc. STAY under-rejected — that is
correct: they join the ask-30 frontier, a decline, never a wrong value.)

**Status.** 🟢 DONE (compiler.cdz). The effect-based diagnostics `compile` (artifact ABI, `Diag` handler) is now
the SHIPPED entry, gate-safe: well-typed → component; provable type error → CDZ0201 (agrees with native); the
ask-30 frontier + unsupported constructs → silent decline (never a false CDZ0201, never a wrong value). The
seed side of diagnostics-via-effects (ask-41/45/46/49/51) is fully exercised end-to-end by the shipped compiler.

---

**🔎 LOOP 4th-probe (2026-07-07 Run 103) — a SECOND residual class the compound-focused analysis missed:
OVER-rejection of well-typed BOOL-PARAMETER programs. This is the opposite failure (false-reject VALID code) and
more urgent than the under-rejects. CONFIDENCE: HIGH (component-check r103, ask-33 runtime-behavior classifier).**
`compile` is now the ACTIVATED artifact-ABI `Diag` handler (`compiler.cdz:2412/2420` → `(compile-artifact
(. a bytes))`), so `component-check` on the emitted component (compiler.cdz 17:25, stable 17:13) exercises the
live check: 79 agree / 102 disagree / 25 soft / 371 decline. Of the 102 disagree, 88 are `native=rejected` comp=ok
(ask-30 under-rejects, expected) but **9 are `native=ok` comp=`diagnostics[CDZ0201]`** — WELL-TYPED programs
native compiles that the check FALSE-REJECTS:

| case | shape |
|---|---|
| `(def (row a b) (if (and a b) 1 0))` conjunction table | `and`/`or` of Bool PARAMS as `if` cond |
| `(def (f b) (if b 10 20))` (applied to true / false) | Bool PARAM as bare `if` cond |
| a boolean parameter forwarded to a conditional result | Bool param through a call into `if` |
| a boolean literal pattern matches a runtime scrutinee | Bool via runtime MATCH scrutinee |
| recursive predicate / self-recursive Bool-returning fn, self-call in then-branch | Bool-returning CALL as cond |
| resolving a head against a prelude symbol rejects a length-mismatched prefix | Bool-returning helper chain |

**Common shape: a Bool whose kind is NOT statically provable at the check point — a function PARAMETER, a CALL
RESULT, or a runtime MATCH SCRUTINEE used in a Bool position.** The compound analysis above used LITERAL Bool
controls (`(+ 1 true)`, `(if true 1 false)`), which carry a known kind, and concluded "Bool is fine." That is an
aggregate that needed re-probing: **literal Bool is fine; parameter/call-result Bool is OVER-rejected.** The check
treats "cannot prove this operand is a Bool" as "type error → CDZ0201" instead of "cannot prove a mismatch → stay
silent" — the CONSERVATIVE principle inverted.

**So ask-53 has TWO residual halves, OPPOSITE in sign:**
- **UNDER-reject** (compound operand mis-labeled `Ki64` → misses genuine rejects like `(+ (list 1) 2)`). Fix:
  `KCompound` + emit at scalar positions.
- **OVER-reject** (this probe: unknown-kind param/call/scrutinee Bool → false CDZ0201 on WELL-TYPED code). Fix: a
  `KUnknown` kind that is NEVER an emit trigger — emit ONLY when BOTH operands have known, mismatched kinds. **This
  is the more urgent half:** an under-reject accepts a bad program (ask-30 territory, already tracked); an
  over-reject REJECTS A GOOD ONE — the corpus's well-typed Bool-param cases score `disagree` the moment `compile`
  ships the record.

**Net:** the lattice needs BOTH `KCompound` (representable-but-mismatched at scalar positions) AND `KUnknown`
(not-positively-known → never emit). Emit iff both operands have KNOWN kinds that mismatch at a scalar-required
position. Acceptance now ALSO includes the 9 Bool-param cases scoring `agree`/`soft` (NOT `diagnostics`). ⚠ This
is the concrete reason `compile` must stay bare-Bytes until BOTH halves land — the activated handler currently
false-rejects 9 well-typed programs (verified live this cycle).
