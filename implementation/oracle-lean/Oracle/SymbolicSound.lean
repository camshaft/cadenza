/-
`Oracle.SymbolicSound` — machine-checked (∀-quantified / decidable) invariants of the symbolic
normalizer's foundations. Unlike the `#guard` spot-checks in `Oracle.Symbolic` (which test specific
values), these hold for EVERY operand / by full evaluation of the classification data. They pin the
facts the normalizer's trap-guarded rewrites rest on:

  • `isConcreteSym` decides EXACTLY the value-shaped constructors (const/ctor/tuple/record) — so a
    match on a symbolic (non-value) scrutinee becomes a symbolic `case`, never a wrongly-decided
    concrete match. Fully characterized below over all nine `SymExpr` constructors.
  • `symToValue?` (the constant-extraction the folder gates on) yields a value on a `const` leaf and
    REFUSES every symbolic leaf (`var`/`app`/`ite`/`proj`/`case` → `none`) — so the folder never treats
    a symbolic operand as constant, a precondition of every constant fold being sound.
  • the trap classification (`arithOps` ∪ `bitwiseOps`) covers EXACTLY the arithmetic/bitwise ops and
    NONE of the comparisons/booleans — so `mayTrap` flags `(/ x 0)` (blocking `if c a a → a`) but not
    `(< x y)` (allowing the collapse). This is what makes dropping a trap-free operand sound
    (`x*0 → 0`, `if c a a → a`, `or a true → true` drop an operand only when it cannot trap).

Scope note — `symToValue?` is now TOTAL (a structural `termination_by sizeOf` def, converted from
`partial` alongside this file), so its equation lemmas exist and `simp [symToValue?]` proves its leaf
facts (below). Still deferred: `mayTrap` / `normalize` remain `partial def`s (Array-closure recursion),
hence OPAQUE in the kernel — no equation lemmas, so `rfl`/`simp`/`unfold` cannot prove even
`mayTrap (.var n) = false`. The same `attach`+`termination_by sizeOf`+`decreasing_by` treatment used
for `symToValue?` unlocks them next (see the vertical log). Likewise the full goal
`denote (normalize e) = denote e` needs the `denote` semantics (with an ambient integer width, since a
symbolic arithmetic `app` is width-erased) and equation lemmas for `normalize`.
-/
import Oracle.Symbolic

namespace Oracle

-- `arithOps`/`bitwiseOps` live in `namespace Oracle.Eval` (mirrors `Oracle.Symbolic`'s `open Eval`).
open Eval

/-! ## `isConcreteSym` is decided exactly on the value-shaped constructors.
`isConcreteSym` is a plain (non-`partial`) `def`, so each arm reduces by `rfl`. Characterizing it over
ALL nine constructors pins the dispatch that keeps symbolic matching sound: a scrutinee is treated as
concrete (statically decidable match) IFF it is one of the four value shapes; every symbolic shape
(var/app/ite/proj/case) is NOT concrete, so its match is deferred to a symbolic `case`. -/

theorem isConcreteSym_const (v : Value) : isConcreteSym (.const v) = true := rfl
theorem isConcreteSym_ctor (tag : ByteArray) (args : Array SymExpr) :
    isConcreteSym (.ctor tag args) = true := rfl
theorem isConcreteSym_tuple (es : Array SymExpr) : isConcreteSym (.tuple es) = true := rfl
theorem isConcreteSym_record (fs : Array (ByteArray × SymExpr)) :
    isConcreteSym (.record fs) = true := rfl

theorem isConcreteSym_var (n : Nat) : isConcreteSym (.var n) = false := rfl
theorem isConcreteSym_app (op : String) (args : Array SymExpr) :
    isConcreteSym (.app op args) = false := rfl
theorem isConcreteSym_ite (c t e : SymExpr) : isConcreteSym (.ite c t e) = false := rfl
theorem isConcreteSym_proj (b : SymExpr) (sel : ByteArray) : isConcreteSym (.proj b sel) = false := rfl
theorem isConcreteSym_case (s : SymExpr) (arms : Array (ByteArray × SymExpr)) :
    isConcreteSym (.case s arms) = false := rfl

/-! ## `symToValue?` extracts a value from a `const` leaf and refuses every symbolic leaf.
`symToValue?` is a total (`termination_by sizeOf`) def, so its equation lemmas discharge these via
`simp`. A `const` leaf extracts its value; a `var` (symbolic input) and every non-value leaf yield
`none` — the folder therefore never mistakes a symbolic operand for a constant (`foldConst?` returns
`none` the moment any operand is symbolic), the precondition that keeps constant folding sound. -/

theorem symToValue?_const (v : Value) : symToValue? (.const v) = some v := by
  simp [symToValue?]
theorem symToValue?_var (n : Nat) : symToValue? (.var n) = none := by
  simp [symToValue?]
theorem symToValue?_app (op : String) (args : Array SymExpr) :
    symToValue? (.app op args) = none := by simp [symToValue?]
theorem symToValue?_ite (c t e : SymExpr) : symToValue? (.ite c t e) = none := by
  simp [symToValue?]
theorem symToValue?_proj (b : SymExpr) (sel : ByteArray) : symToValue? (.proj b sel) = none := by
  simp [symToValue?]
theorem symToValue?_case (s : SymExpr) (arms : Array (ByteArray × SymExpr)) :
    symToValue? (.case s arms) = none := by simp [symToValue?]

/-! ## Trap classification: `arithOps` ∪ `bitwiseOps` = exactly the trapping ops.
`arithOps`/`bitwiseOps` are plain `List String` data, so membership is decidable — these `decide`
facts pin that `mayTrap`'s `arithOps.contains op || bitwiseOps.contains op` guard fires on ALL
arithmetic/bitwise ops (which can trap: div-by-zero, overflow, shift-out-of-range) and on NONE of the
comparisons/booleans (which do not themselves trap). Together with the (deferred) leaf facts this is
what makes the trap-guarded operand-dropping rewrites sound in BOTH directions:
  • arithmetic classified trapping ⇒ `mayTrap (/ x 0) = true` ⇒ `if (/ x 0) a a` is NOT collapsed.
  • comparisons/booleans classified non-trapping ⇒ a comparison condition is droppable when a branch
    is duplicated / an operand is redundant (`if (< x y) a a → a`, `or (< x y) true → true`). -/

/-- Every arithmetic op is classified as trapping. -/
theorem arithOps_covers_arithmetic :
    (["+", "-", "*", "/", "%"].all (arithOps.contains ·)) = true := by decide

/-- No comparison op is in the arithmetic-trap set. -/
theorem arithOps_excludes_comparisons :
    (["=", "<", ">", "<=", ">="].any (arithOps.contains ·)) = false := by decide

/-- No boolean op is in the arithmetic-trap set. -/
theorem arithOps_excludes_booleans :
    (["and", "or", "not"].any (arithOps.contains ·)) = false := by decide

/-- No comparison/boolean op is in the bitwise-trap set either — so the FULL trapping set
(`arithOps ∪ bitwiseOps`) excludes every comparison/boolean, the ops a trap-guarded rewrite may drop. -/
theorem bitwiseOps_excludes_comparisons_and_booleans :
    (["=", "<", ">", "<=", ">=", "and", "or", "not"].any (bitwiseOps.contains ·)) = false := by decide

end Oracle
