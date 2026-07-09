## 70. 🟢 STRING heap-eq FIXED (incl. payload-extracted) — residual: runtime COMPOUND (sum/tuple/record) eq + let-aliased operand

**⚡UPDATE 2026-07-08 (09:40 stable): STRING heap-eq now covers everything the rewrite's `resolve` needs.**
The compiler agent extended the heap-walk to a String operand extracted from a variant PAYLOAD:
- ✅ `(= a b)` on two bare String PARAMETERS → runs.
- ✅ String bound from a SUM-VARIANT PAYLOAD `= param` (both `Ast.Name` AND a user sum) → **now runs**. Corpus
  "a runtime string bound from a sum payload compares equal to a string parameter" → **PASS**. This is
  exactly `resolve`'s `name-head-is` shape, so the **full `resolve-program` pipeline now compiles
  byte-identically** (verified: a hand-built `Ast` → the 89-byte scalar component, runs→42). ✅ **ask-70 no
  longer blocks the rewrite front rung.**

**Residual (still declines — NOT needed for Phase 0–2 resolve; a later-phase concern):**
- ❌ `(let ((x s)) (= x s))` — an operand aliased to the same source via `let` (two DISTINCT params via let
  work; the gap is the same-source alias).
- ❌ Two RUNTIME COMPOUND values (sum/tuple/record built at run time) — corpus "two runtime sum values
  compare equal by a heap walk" remains `todo`. The rewrite will need this once it compares whole decoded
  `Ast` sub-trees / records for structural equality (Phase 3+), but not for scalar-`main` resolution.

*(Historical) What `resolve` needed (NOW FIXED):* the payload case — `(= <String from an Ast.Name payload>
<expected-name>)`. The heap-walk now extends to a heap-String operand extracted
from a variant payload (and, for later phases, to runtime compound values).

---

*(Original report, now the residual-gap description.)*

**⚡ALREADY PINNED IN THE CORPUS (this ask is the rewrite-blocker framing, not a new gap).** The behavior
gate already carries `todo` cases for exactly this decline — `spec/semantics/03-equality-and-observation.sexp`:
"two runtime strings compare equal by their contents" (`(eq2 "foo" "foo")` → true), its unequal companion,
the literal-fold control, and "two runtime sum values compare equal by a heap walk". So the compiler agent
can reproduce it immediately by running the behavior gate and looking for `[runtime compound equality (heap
walk) not yet emitted]`. This ask adds only WHY it is load-bearing NOW: it is the second front-end blocker of
the compiler rewrite (`resolve` compares a decoded name against a stored/expected name — both runtime).

**Status: BLOCKING (the SECOND rewrite front-end gap, independent of ask-69).** After the compiler rewrite
decodes to an `Ast` (blocked on ask-69), `resolve` must compare decoded `Ast.Name` strings against expected
keywords — e.g. "is this form's head the name `def` / `main` / `module`?". That comparison is
`(= <String pulled from an Ast.Name payload> <the expected name>)`. When the expected name is **also a
runtime value** (a parameter, or itself another heap String), the seed declines:
**"runtime compound equality (heap walk) not yet emitted."** Native declines it too — a genuine capability
limit, not a mine-vs-native gap.

**The precise discriminator (verified on the stable seed, `emit`):**

```
; DECLINES — a String from an Ast.Name payload (heap) compared to a String PARAMETER (heap), neither literal:
(module m
  (def (f h s) (match h ((Ast.Name nm) (= nm s)) ((Ast.Int n) false) (…other Ast arms false…)))
  (def (main) (if (f (Ast.Name "def") "def") 1 0)))
;   native: decline "runtime compound equality (heap walk) not yet emitted"

; COMPILES — heap String from an Ast.Name payload = a LITERAL (one side folds), 4011 bytes:
(module m
  (def (f h) (match h ((Ast.Name nm) (= nm "def")) (…other Ast arms false…)))
  (def (main) (if (f (Ast.Name "def")) 1 0)))

; COMPILES — two BARE String params `(= a b)` (the scalar-String path), 100 bytes:
(module m (def (f a b) (= a b)) (def (main) (if (f "x" "x") 1 0)))
```

So the three cases split cleanly:
- heap-String `=` **literal** → works (the literal side const-folds; the existing string-producer fold).
- two bare **String params** `(= a b)` → works (a scalar-tier String path).
- **heap value `=` heap value, neither a literal** (at least one extracted from a variant payload / genuinely
  on the heap, the other a runtime String) → **declines** — the runtime heap-walk equality comparator is not
  emitted.

**What the compiler needs.** A RUNTIME equality that walks two heap values (String, and more generally the
persistent compound/CHAMP/rope representations) and compares by canonical content — the "heap walk" the
decline names. The immediate need is **String = String at runtime with neither side a literal** (comparing a
decoded name against an expected-name value); the general need is runtime compound equality (tuples/records/
lists/sums by structure), which the rewrite will also hit as it matches decoded `Ast` sub-trees.

**Why it BLOCKS the rewrite.** `resolve` is fundamentally "dispatch on the decoded head name." If the
expected keyword must be a literal on the RHS of every comparison, resolution can't be data-driven (e.g.
can't look a decoded name up in an environment of `(name → binding)` pairs, since the env's names are
runtime Strings). Concretely, `resolve-program` on a hand-built `Ast` (bypassing the ask-69 decode blocker)
declines here, in the `name-head-is` helper. Per the approved rewrite plan, this is documented and BLOCKED
on — not worked around (a literal-only comparison would force a hard-coded keyword ladder, the anti-pattern
the rewrite exists to remove).

**Workaround that is NOT taken (and why).** One *could* keep every keyword comparison as `(= nm "literal")`
with the literal always on one side. That works for a fixed keyword set but defeats the rewrite's premise
(names resolve to bindings in an environment; a decoded name is compared against *stored* names). So we
document the gap and wait for runtime heap equality rather than contort `resolve` into a literal-only ladder.

**Priority.** 🔴 HIGH — with ask-69 it is one of the two front-end blockers of the compiler rewrite. Even
once ask-69 lands (runtime `Ast.decode`), `resolve` cannot compare decoded names without this.

**Acceptance signal.** The first (DECLINES) reproducer above `emit`s a valid component; more generally,
`(= s1 s2)` for two runtime Strings (neither literal) compiles and computes content equality, and a
runtime compound `(= x y)` over two heap values of the same shape compiles. Related: ask-69 (the decode
blocker upstream of this), ask-60 (heap-value *emission* — this is the *comparison* dual), and the memory
[[runtime-compound-element-compound-output-declines]] / the "runtime-compound-eq-heapwalk unrealized" note
in the adversarial-loop declines.
