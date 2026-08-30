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

  • `mayTrap` is FALSE on both leaves (`const`/`var`) — the trap-freedom fact that directly licenses
    dropping a constant or variable operand in a trap-guarded rewrite (`x*0 → 0`, `if x a a → a`).
  • `normalize` is the IDENTITY on a symbolic input (`.var n`) — a variable has no sound rewrite, so
    the canonicalizer leaves it untouched (the base case the whole recursion bottoms out on).

Scope note — `symToValue?`, `mayTrap`, and `normalize` are now all TOTAL (structural
`termination_by sizeOf` defs, converted from `partial` alongside this file; `normalize`'s `app`-arm
algebraic identities were split into `normalizeAppIdentities` so its matcher stays simple enough for
equation-lemma generation). Their equation lemmas exist and `simp` proves the leaf facts below. The
REMAINING work is the deeper normalizer theorems — most of all the soundness goal
`denote (normalize e) = denote e` (the canonicalizer preserves meaning ⇒ `symEquiv` is never a false
`proven`), which needs the `denote` semantics (with an ambient integer width, since a symbolic
arithmetic `app` is width-erased). Totality of all three is the foundation that unblocks it.
-/
import Oracle.Symbolic

namespace Oracle

-- `arithOps`/`bitwiseOps` live in `namespace Oracle.Eval` (mirrors `Oracle.Symbolic`'s `open Eval`).
open Eval

/-! ## `denote` — the concrete meaning of a `SymExpr` under a valuation (soundness spec).
`denote ρ w e` evaluates the symbolic expression `e` to a concrete `Outcome`, with each symbolic
variable `.var n` bound to the value `ρ n` and integer arithmetic performed at ambient width `w`. It is
the SEMANTICS the normalizer must preserve: the capstone soundness goal is
`denote ρ w (normalize e) = denote ρ w e` for all ρ, w — i.e. `normalize` never changes what a program
computes on any input, so a `proven`-equivalent verdict is never false.

Faithfulness (so the eventual theorem is MEANINGFUL, not a toy): the `.app` case REUSES the exact ops
the oracle/normalizer use — `foldConst?` for the comparison/boolean/float/equality ops (byte-identical
to what `normalize` folds), and `evalArithOp` at width `w` for the deferred integer arithmetic
(`+ - * / %`, with its real overflow-trap semantics). This alignment with `normalize`'s own folding is
what will make the soundness induction go through. Analyzable fragment only (var/const/scalar-app/ite);
compound shapes (tuple/record/ctor/proj/case) are `unsupported` here — a later increment. -/
/-- Combine a UNARY op over its (already-denoted) operand outcome — reuses `foldConst?` (e.g. `not`). -/
def denoteUnary (op : String) (oa : Outcome) : Outcome :=
  match oa with
  | .value va => (match foldConst? op #[.const va] with
                  | some v => .value v
                  | none => .unsupported "denote: unmodeled unary op")
  | o => o

/-- Combine a BINARY op over its (already-denoted) operand outcomes, left-to-right trap propagation.
Value operands fold via `foldConst?` (comparison/boolean/float/equality — byte-identical to `normalize`)
or, for the deferred integer arithmetic `foldConst?` leaves unfolded, via the real `evalArithOp` at
width `w` (its true overflow-trap semantics). NON-recursive — split out of `denote` so `denote`'s own
matcher stays simple enough for equation-lemma generation. -/
def denoteBinary (op : String) (w : IntTy) (oa ob : Outcome) : Outcome :=
  match oa, ob with
  | .value va, .value vb =>
    (match foldConst? op #[.const va, .const vb] with
     | some v => .value v
     | none => (match va, vb with
                | .int x, .int y => if arithOps.contains op then evalArithOp op x y w
                                    else .unsupported "denote: unmodeled binary op"
                | _, _ => .unsupported "denote: non-integer operands for a deferred op"))
  | .trap t, _ => .trap t
  | _, .trap t => .trap t
  | .diverges, _ => .diverges
  | _, .diverges => .diverges
  | _, _ => .unsupported "denote: unmodeled operand outcome"

/-- Dispatch a denoted application on its arity (unary/binary; else unsupported). NON-recursive, kept
OUT of `denote` (whose WF-recursive equation compiler rejects array-literal patterns). Uses a
dependent-`if` on `oargs.size` with proof-carrying indexing (`oargs[i]'h`) rather than `#[oa]`/`#[oa,ob]`
literal patterns, because those literal-array matches defeat both `split` and `simp`'s equation
generation — this size form lets the soundness proofs (`denoteApp_ne_trap`) case via `by_cases` and get
operand membership from `Array.getElem_mem`. -/
def denoteApp (op : String) (w : IntTy) (oargs : Array Outcome) : Outcome :=
  if h1 : oargs.size = 1 then denoteUnary op (oargs[0]'(by omega))
  else if h2 : oargs.size = 2 then denoteBinary op w (oargs[0]'(by omega)) (oargs[1]'(by omega))
  else .unsupported "denote: unsupported operator arity"

def denote (ρ : Nat → Value) (w : IntTy) : SymExpr → Outcome
  | .const v => .value v
  | .var n => .value (ρ n)
  | .ite c t e =>
    match denote ρ w c with
    | .value (.bool true) => denote ρ w t
    | .value (.bool false) => denote ρ w e
    | .value _ => .unsupported "denote: non-boolean ite condition"
    | o => o
  | .app op args => denoteApp op w (args.attach.map (fun x => denote ρ w x.val))
  | _ => .unsupported "denote: unmodeled construct (compound shape — later increment)"
termination_by e => sizeOf e
decreasing_by
  all_goals simp_wf
  all_goals first
    | omega
    | (have h := Array.sizeOf_lt_of_mem x.property; omega)

-- Sanity checks that `denote` computes the expected concrete outcomes (Lean-native, justified: this is
-- a ∀-inputs metatheorem SPEC beyond corpus reach). Leaves, arithmetic, overflow, comparison, if-select.
#guard (denote (fun _ => .unit) defaultIntTy (.const (.int 5)) == .value (.int 5))
#guard (denote (fun _ => .int 7) defaultIntTy (.var 0) == .value (.int 7))
#guard (denote (fun _ => .unit) defaultIntTy (.app "+" #[.const (.int 2), .const (.int 3)]) == .value (.int 5))
#guard (denote (fun _ => .unit) defaultIntTy (.app "<" #[.const (.int 2), .const (.int 3)]) == .value (.bool true))
#guard (denote (fun _ => .unit) defaultIntTy
          (.app "+" #[.const (.int 9223372036854775807), .const (.int 1)]) == .trap "overflow")
#guard (denote (fun _ => .unit) defaultIntTy
          (.ite (.const (.bool true)) (.const (.int 1)) (.const (.int 2))) == .value (.int 1))

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

/-! ## `mayTrap` is false on the leaves.
`mayTrap` is a total (`termination_by sizeOf`) def; its `const`/`var` arms are literally `false`, so
`simp` discharges these. A leaf provably cannot trap — the fact that makes dropping a constant or
variable operand in a trap-guarded rewrite sound (`x*0 → 0` when `x` is a leaf, `if x a a → a` for a
variable/constant condition, `or a true → true` when `a` is a leaf). -/

theorem mayTrap_const (v : Value) : mayTrap (.const v) = false := by simp [mayTrap]
theorem mayTrap_var (n : Nat) : mayTrap (.var n) = false := by simp [mayTrap]

/-- `mayTrap` propagates through an `ite` structurally (a conditional traps iff its condition or either
branch can trap) — the recursion the equal-branch collapse guard reads: `if c a a → a` is sound only
when `mayTrap c = false`, which by this fact is subsumed by `mayTrap (.ite c a a) = false`. -/
theorem mayTrap_ite (c t e : SymExpr) :
    mayTrap (.ite c t e) = (mayTrap c || mayTrap t || mayTrap e) := by simp [mayTrap]

/-- `mayTrap` on a projection is exactly the base's trap-ness. -/
theorem mayTrap_proj (b : SymExpr) (sel : ByteArray) : mayTrap (.proj b sel) = mayTrap b := by
  simp [mayTrap]

/-- `mayTrap` on a symbolic `case` traps iff the scrutinee or any arm body can — the recursion a
symbolic-match's trap analysis reads. -/
theorem mayTrap_case (s : SymExpr) (arms : Array (ByteArray × SymExpr)) :
    mayTrap (.case s arms) = (mayTrap s || arms.attach.any (fun x => mayTrap x.val.2)) := by
  simp [mayTrap]

/-! ### `denote` trap-freedom building blocks (toward `mayTrap` soundness).
These pin the operand-combiners' trap behavior — the crux of the eventual `mayTrap`-soundness theorem
(`mayTrap e = false → denote never traps e`, the justification for every trap-guarded rewrite like
`x*0 → 0`, `if c a a → a`, `or a true → true`) and reusable for the `denote (normalize e) = denote e`
capstone. The only way `denote` produces a trap on an `.app` is `evalArithOp` inside `denoteBinary`;
these lemmas show that path is unreachable when the op is non-arithmetic and the operands don't trap. -/

/-- `denoteUnary` never traps when its operand outcome does not (`foldConst?` yields a `.value`; a
non-`.value` operand is passed through). -/
theorem denoteUnary_ne_trap (op : String) (oa : Outcome) (k : String)
    (ha : ∀ t, oa ≠ .trap t) : denoteUnary op oa ≠ .trap k := by
  intro h
  cases oa with
  | value va => simp only [denoteUnary] at h; split at h <;> simp_all
  | trap t => exact ha t rfl
  | diverges => simp [denoteUnary] at h
  | unsupported r => simp [denoteUnary] at h
  | errReturn v => simp [denoteUnary] at h

/-- `denoteBinary` never traps when its op is NOT arithmetic and neither operand outcome traps: value
operands fold via `foldConst?` (a `.value`) or, since `op ∉ arithOps`, fall to `.unsupported` (the
`evalArithOp` trap path is guarded out); non-value operands are non-traps by hypothesis. -/
theorem denoteBinary_ne_trap (op : String) (w : IntTy) (oa ob : Outcome) (k : String)
    (hop : arithOps.contains op = false)
    (ha : ∀ t, oa ≠ .trap t) (hb : ∀ t, ob ≠ .trap t) :
    denoteBinary op w oa ob ≠ .trap k := by
  intro h
  cases oa with
  | value va =>
    cases ob with
    | value vb =>
      simp only [denoteBinary] at h
      split at h
      · simp_all
      · split at h <;> simp_all
    | trap t => exact hb t rfl
    | diverges => simp [denoteBinary] at h
    | unsupported r => simp [denoteBinary] at h
    | errReturn v => simp [denoteBinary] at h
  | trap t => exact ha t rfl
  | diverges => cases ob <;> first | exact hb _ rfl | simp_all [denoteBinary]
  | unsupported r => cases ob <;> first | exact hb _ rfl | simp_all [denoteBinary]
  | errReturn v => cases ob <;> first | exact hb _ rfl | simp_all [denoteBinary]

/-! ## `normalize` is the identity on a symbolic input.
`normalize` is a total (`termination_by sizeOf`) def; its `.var` arm is literally `.var n`, so `simp`
discharges this via the equation lemma. A symbolic input has no sound rewrite — the canonicalizer
leaves it untouched (the base case the recursion terminates on). -/

theorem normalize_var (n : Nat) : normalize (.var n) = .var n := by simp [normalize]

/-! ### `normalize` recurses STRUCTURALLY through the value-shaped constructors.
For the non-`app`/non-`ite` compound shapes `normalize` is a pure congruence — it rebuilds the same
constructor with each child normalized, introducing no fold/rewrite (those live only in the `.app`
algebraic identities and the `.ite` fold+collapse). Pinning these equations (a) guards the recursion
SHAPE against a future refactor and (b) supplies the congruence lemmas an eventual
`denote (normalize e) = denote e` proof needs for these constructors. `simp [normalize]` discharges each
via the (now-total) equation lemmas. -/

theorem normalize_proj (b : SymExpr) (s : ByteArray) : normalize (.proj b s) = .proj (normalize b) s := by
  simp [normalize]
theorem normalize_tuple (es : Array SymExpr) :
    normalize (.tuple es) = .tuple (es.attach.map (fun x => normalize x.val)) := by simp [normalize]
theorem normalize_ctor (tag : ByteArray) (args : Array SymExpr) :
    normalize (.ctor tag args) = .ctor tag (args.attach.map (fun x => normalize x.val)) := by
  simp [normalize]
theorem normalize_case (s : SymExpr) (arms : Array (ByteArray × SymExpr)) :
    normalize (.case s arms)
      = .case (normalize s) (arms.attach.map (fun x => (x.val.1, normalize x.val.2))) := by
  simp [normalize]

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

/-- Bridge to the `.app` case of `mayTrap` soundness: a denoted application never traps when the op is
non-arithmetic and every operand outcome is trap-free — `denoteApp` dispatches to `denoteUnary` /
`denoteBinary` (both trap-free by the lemmas above) or to `.unsupported`. -/
theorem denoteApp_ne_trap (op : String) (w : IntTy) (oargs : Array Outcome) (k : String)
    (hop : arithOps.contains op = false)
    (hall : ∀ o ∈ oargs, ∀ t, o ≠ .trap t) :
    denoteApp op w oargs ≠ .trap k := by
  unfold denoteApp
  split
  · exact denoteUnary_ne_trap op _ k (hall _ (Array.getElem_mem _))
  · split
    · exact denoteBinary_ne_trap op w _ _ k hop
        (hall _ (Array.getElem_mem _)) (hall _ (Array.getElem_mem _))
    · intro hc; exact Outcome.noConfusion hc

theorem mayTrap_sound (ρ : Nat → Value) (w : IntTy) (e : SymExpr) :
    mayTrap e = false → ∀ k, denote ρ w e ≠ .trap k := by
  induction e using denote.induct (ρ := ρ) (w := w) <;> intro h <;>
    first
      | (intro k; simp_all [denote, mayTrap]; done)
      | (rename_i op args ih
         simp only [mayTrap, Bool.or_eq_false_iff] at h
         obtain ⟨⟨ha, _⟩, hany⟩ := h
         intro k
         simp only [denote]
         apply denoteApp_ne_trap op w _ k (by simpa using ha)
         intro o ho
         obtain ⟨y, hy, rfl⟩ := Array.mem_map.1 ho
         obtain ⟨i, hi, hval⟩ := Array.getElem_of_mem hy
         simp only [Array.any_eq_false] at hany
         have hmt : mayTrap y.val = false := by have := hany i hi; rw [hval] at this; simpa using this
         exact ih y hmt)

end Oracle
