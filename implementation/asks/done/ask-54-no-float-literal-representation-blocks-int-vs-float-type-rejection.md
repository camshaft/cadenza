## 54. 🟠 (compiler.cdz — MINE, a genuine feature gap, NOT to be worked around) The reader has no FLOAT-LITERAL representation, so it cannot prove an int-vs-float type mismatch (native's CDZ0201/CDZ0301)

**Context.** After the coded-diagnostics + out-of-range-literal + dup-field + malformed-`let` landings, `component-check`
over `spec/semantics` is at **105 agree / 1 disagree / 25 soft / 448 decline / 204 skip** (value-harness 0 hard / 0
error). The SINGLE remaining `disagree` is:

    a conditional with integer and floating-point branches is a type error
      input:  (if true 1 3.5)
      native: rejected CDZ0201 "conditional branches have different types"
      mine:   component ran → "1"  (compiled a program native rejects)

**Root cause (traced, two compounding facts).**
1. **No float representation.** A float literal (`3.5`) arrives as CBOR major-7, info 25/26/27 (`0xF9`/`0xFA`/`0xFB`).
   The reader (`read-node`, major-7 arm) collapses EVERY non-bool major-7 to the generic unsupported marker `"?"` →
   `PUnknown` → `(KError 0)` DECLINE. So a float is INDISTINGUISHABLE from any other unsupported construct. Its
   check-kind (`ck-of`) is therefore `CKUnk`, and `ck-provably-mismatch (KConst 1) (KError 0)` = false — the
   int-vs-float branch mismatch is NOT provable.
2. **`fold-if` discards a dead branch.** `(if true 1 3.5)` has a constant condition, so `fold-if-go` folds to the
   then-branch `1`, DROPPING the else-branch `KError` entirely. Even the decline-trap is gone, so mine runs to `1`.

Native, by contrast, type-checks BOTH branches (int vs float = different types) BEFORE it evaluates the constant
condition, so it rejects regardless of which branch the condition selects.

**Why this is NOT worked around (per the loop discipline).** A bare float `3.5` COMPILES under native (`VALID`, 96
bytes) — it is a legitimate value, so for THIS compiler (which has no float codegen) it is correctly a DECLINE, not an
error. The int-vs-float REJECTION only arises from the COMBINATION (an int operand/branch beside a float one). To
detect that, the reader needs a float literal that is:
  - a DECLINE in codegen (this compiler cannot emit float ops — a bare float must still trap-decline, matching the
    honest frontier), BUT
  - PROVABLY-FLOAT in the check kind, so `ck-provably-not-i64` / `ck-provably-not-bool` / `ck-provably-mismatch` fire
    against an int/bool neighbor.
Overloading the existing `(KError 0)` to mean "float" would POISON the decline path — every unsupported construct
shares that marker, so a float-kind claim on `KError 0` would mis-kind strings, lists, unit, unbound names, etc.
That is exactly the contortion the discipline forbids. The correct fix is a NEW, distinct representation.

**The fix (a real feature, when scheduled).** Add a `KFloat` Core node (payload irrelevant to the check — the raw
bytes or a placeholder; this compiler never emits it) fed by a new `NFloat` surface node from the major-7 float arm
of `read-node`. Then:
  - `ck-of (KFloat _)` → a new `CKFloat` check-kind (extend `CKind` to `CKi64 | CKBool | CKFloat | CKUnk`);
  - `ck-provably-not-i64` / `ck-provably-not-bool` treat `CKFloat` as provably-not (a concrete non-scalar-int kind);
  - `ck-provably-mismatch` returns true for `CKFloat` vs `CKi64`/`CKBool` (concrete distinct kinds);
  - codegen (`lower`/`compile-core`) still DECLINES `KFloat` (→ `unreachable`), so a bare float stays a decline;
  - the `if`-branch, arith-operand, and comparison-operand checks then emit the right code — CDZ0201 for the
    conditional-branch and arith mismatch, **CDZ0301** for an ordering/comparison mismatch (native uses CDZ0301 =
    "numeric types do not silently promote" for `(< 1 3.5)` / `(+ 1 3.5)`; verified below). That means the coded-
    diagnostics channel needs a `301` path too (trivial — `code-string`/`code-message` already map by code).
  - `fold-if` must ALSO not silently drop a dead branch that carries a `KFloat`/`KError` the check would have
    flagged — OR (cleaner) the check runs pre-fold (it already does: `check-funcs` walks the resolved tree before
    `fold-funcs`), so once `ck-of` proves the float mismatch, the diagnostic is emitted regardless of the fold. The
    fold dropping the branch only affects the emitted BYTES (a decline stub), not the diagnostic — so fixing `ck-of`
    alone closes the disagree. (The fold-discards-dead-branch behavior is then benign: the program is already
    flagged ill-typed; its bytes are a decline stub, which is fine.)

**Native ground-truth (compile-run on the reference seed, 2026-07-07):**
| program            | native verdict                                             |
|--------------------|------------------------------------------------------------|
| `3.5`              | ✅ compiles (VALID 96 bytes) — a bare float is a value      |
| `(if true 1 3.5)`  | 🔴 CDZ0201 "conditional branches have different types"     |
| `(+ 1 3.5)`        | 🔴 CDZ0301 "numeric types do not silently promote"         |
| `(< 1 3.5)`        | 🔴 CDZ0301 "ordering between values of different types"    |
| `(= 1 3.5)`        | 🔴 CDZ0201 "comparison between values of different types"   |

**Acceptance signal.** With `KFloat` + `CKFloat`: `(if true 1 3.5)` → CDZ0201 (the last disagree → agree); the three
int-float arith/ordering cases (currently safe DECLINEs) → their native codes (0201/0301), moving decline→agree; a
bare `3.5` STAYS a decline (native compiles it, mine has no float codegen — never a false CDZ). Net: the disagree
count goes to 0 for the type-rejection frontier reachable without full compound support; the only remaining
native-rejects/mine-declines are the genuine compound gaps (records/tuples/maps/sum-patterns — ask-13).

**Status.** 🟠 compiler.cdz feature (mine, scheduled). This is the natural completion of the coarse-kind lattice
(ask-53 added `CKind`; this adds `CKFloat` to it) and the coded-diagnostics channel (adds the `301` code path). It
is the LAST reachable type-rejection before the compound frontier (ask-13). Deliberately documented rather than
rushed, because a correct float marker must be distinct from the `(KError 0)` decline — a shortcut there would
regress the decline path. Related: ask-53 (the `CKind` lattice this extends), ask-30 (the rejection frontier),
ask-13 (the compound gaps that remain after this).

---

## ✅ RESOLVED 2026-07-07 (compiler.cdz — MINE) — `KFloat`/`CKFloat` landed; **byte-gate went GREEN (0 disagree)**

Implemented exactly as designed: new `NFloat` surface node (reader major-7 info≥25 arm) → `Core.KFloat` (a
CHECK-ONLY node: `lower` DECLINES it to `unreachable`, so a bare float stays a decline — native compiles it, this
compiler has no float codegen). Added `CKFloat` to the `CKind` lattice; `ck-of (KFloat) = CKFloat`; generalized
`ck-eq`/`ck-concrete`/`ck-provably-not-i64`/`ck-provably-not-bool`/`ck-provably-mismatch` to the 3 concrete kinds.
Wired `KFloat` through all 12 exhaustive `Core` matches + the `Node` match (`node-int`). Position-aware CODE:
`check-arith` and a NEW `check-order` (the four ordering ops, split from equality's `check-cmp`) emit via
`numeric-mismatch-code` → **CDZ0301** when a float is involved (native's numeric-non-promotion code), CDZ0201 for a
Bool mismatch; equality/`if`-branch/`if`-cond/`not` stay CDZ0201.

**🔑 ROOT BUG this uncovered (the real blocker): `code-string` silently collapsed every non-210 code to CDZ0201.**
The check was emitting 301 correctly ALL ALONG, but `code-string`/`code-message` only had a `210` case + a CDZ0201
fallback — so `301 → "CDZ0201"` (and my 888/900 debug probes ALSO displayed as CDZ0201, which is what made this so
confusing to trace). A diagnostic is displayed THROUGH `code-string`, so any code without an explicit case is
invisible. Added the `301 → CDZ0301` case. ⚠ LESSON (now a code comment): every code the check emits MUST have a
`code-string` case or it collapses to CDZ0201.

**Gate deltas:** component-check **106 → 120 agree, 14 → 0 disagree — PASS** (the byte-level self-hosting gate is
GREEN for the first time; 25 soft, 434 decline). Value-harness **0 hard / 0 error**. 0 false-rejects, 0 crashes.
Also RESOLVES **ask-55** (the sibling's float-crash regression): a bare float now cleanly declines — all 12
Core-match arms handle `KFloat` and `lower` emits `unreachable`, so there is no non-exhaustive-match trap.

**Status.** 🟢 DONE (compiler.cdz). The int-vs-float type-rejection frontier is closed; the only remaining
native-rejects/mine-declines are the compound gaps (records/tuples/maps/sum-patterns — ask-13), which are honest
declines (component-check counts them as `decline`, not `disagree`).
