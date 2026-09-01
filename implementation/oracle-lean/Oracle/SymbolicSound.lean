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
the SEMANTICS the normalizer must preserve: the capstone soundness goal is that `normalize` never changes
what a program computes on any input, so a `proven`-equivalent verdict is never false.

⚠ CAPSTONE FORM (design note, established while assembling the induction): the goal is proven in
VALUE-FORM — `denote ρ w e = .value v → denote ρ w (normalize e) = .value v` — NOT the naive full equality
`denote (normalize e) = denote e`. The full equality is FALSE on ill-typed `e`: `normalize`'s `.ite`
MATERIALIZE (`if c true false → c`, #6450) fires unconditionally on the branch shape, so for a NON-bool `c`
`denote (.ite c true false) = .unsupported` (non-bool condition) while `denote (normalize …) = denote c`
(a value) — they differ. Value-form neatly excludes this: a non-bool `c` makes the `ite` non-value, so the
hypothesis `denote e = .value v` never fires there (equivalently, equality holds under well-typedness, which
value-form encodes implicitly). All the `_step` lemmas are value-form for this reason; the equality-form
case lemmas that DO hold unconditionally (var/const/tuple/record/ctor/proj/case, ite fold-select) are
strictly stronger and feed the value-form induction directly (equality ⟹ the value-form implication).

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
  -- a `.const` denotes to its SEMANTIC value; a float literal is canonicalized to its `.f64` (its actual
  -- runtime value) exactly as `normalize` does, so `denote (normalize e) = denote e` holds STRUCTURALLY
  -- (a float literal and a computed `.f64` of the same value denote identically).
  | .const v => .value (match Value.asF64? v with | some f => .f64 f | none => v)
  | .var n => .value (ρ n)
  | .ite c t e =>
    match denote ρ w c with
    | .value (.bool true) => denote ρ w t
    | .value (.bool false) => denote ρ w e
    | .value _ => .unsupported "denote: non-boolean ite condition"
    | o => o
  | .app op args => denoteApp op w (args.attach.map (fun x => denote ρ w x.val))
  -- a TUPLE denotes to a tuple VALUE: each element is stored via `outcomeToValue` (a trapping/diverging
  -- element becomes a deferred `poison`, surfaced only when that element is OBSERVED) — byte-identical to
  -- `evalNode`'s tuple construction (spec core: "a trap occurs when observed"). So tuple CONSTRUCTION is a
  -- value (never traps here), matching the concrete evaluator; coverage for compound-valued programs.
  | .tuple es => .value (.tuple (es.attach.map (fun x => outcomeToValue (denote ρ w x.val))))
  -- a RECORD denotes to a record VALUE, each field stored via `outcomeToValue` (trapping field → deferred
  -- `poison`, "trap-when-observed"), byte-identical to `evalNode`'s record construction. Like `.tuple`,
  -- CONSTRUCTION is a value (never traps here) so it does not complicate `mayTrap_sound`. (Projection /
  -- `case` — which OBSERVE and can trap — are a later increment; they interact with `mayTrap`.)
  | .record fs => .value (.record (fs.attach.map (fun x => (x.val.1, outcomeToValue (denote ρ w x.val.2)))))
  | _ => .unsupported "denote: unmodeled construct (compound shape — later increment)"
termination_by e => sizeOf e
decreasing_by
  all_goals simp_wf
  all_goals first
    | omega
    | (have h := Array.sizeOf_lt_of_mem x.property; omega)
    | (rcases x with ⟨⟨k, e⟩, hmem⟩; have h := Array.sizeOf_lt_of_mem hmem; simp_all; omega)

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
theorem normalize_record (fs : Array (ByteArray × SymExpr)) :
    normalize (.record fs) = .record (fs.attach.map (fun x => (x.val.1, normalize x.val.2))) := by
  simp [normalize]
theorem normalize_ctor (tag : ByteArray) (args : Array SymExpr) :
    normalize (.ctor tag args) = .ctor tag (args.attach.map (fun x => normalize x.val)) := by
  simp [normalize]
theorem normalize_case (s : SymExpr) (arms : Array (ByteArray × SymExpr)) :
    normalize (.case s arms)
      = .case (normalize s) (arms.attach.map (fun x => (x.val.1, normalize x.val.2))) := by
  simp [normalize]

/-! ### `normalize`-`app` branch equations (conditional — the hypothesis concretizes the `foldConst?`
match, dodging the bare-restatement match-motive wall). `normalize` on an `.app` first normalizes the
args, then FOLDS them (`foldConst? = some v` ⇒ `.const v`) or applies the algebraic identities
(`foldConst? = none` ⇒ `normalizeAppIdentities`). These expose the two `.app` branches for the capstone
`.app` case (fold branch → `denoteBinary_fold`; identity branch → the arith value-characterizations). -/
theorem normalize_app_fold (op : String) (args : Array SymExpr) (v : Value)
    (h : foldConst? op (args.attach.map (fun x => normalize x.val)) = some v) :
    normalize (.app op args) = .const v := by
  simp only [normalize, h]

theorem normalize_app_ident (op : String) (args : Array SymExpr)
    (h : foldConst? op (args.attach.map (fun x => normalize x.val)) = none) :
    normalize (.app op args) = normalizeAppIdentities op (args.attach.map (fun x => normalize x.val)) := by
  simp only [normalize, h]

/-! ### `normalize`-`ite` fold-SELECT equations: a condition that normalizes to a bool literal collapses.
When `normalize c` folds to `.const (.bool b)`, the whole `ite` collapses to the normal form of the
selected branch (`normalize`'s `.ite` arm short-circuits before the materialize/collapse/rebuild logic).
These are the structural half of the capstone `.ite` fold-select sub-case; paired with the IH on the
selected branch (and the `.ite` value-inversion `denote_ite_value_inv`), they carry the meaning across. -/
theorem normalize_ite_condTrue (c t e : SymExpr) (hc : normalize c = .const (.bool true)) :
    normalize (.ite c t e) = normalize t := by
  simp only [normalize, hc]

theorem normalize_ite_condFalse (c t e : SymExpr) (hc : normalize c = .const (.bool false)) :
    normalize (.ite c t e) = normalize e := by
  simp only [normalize, hc]

/-- CAPSTONE `.ite` fold-select cases (full-EQUALITY, condition folds to a bool literal). When
`normalize c = .const (.bool true)`, `normalize` selects `t`; and `ihc` forces the actual `denote c` to
`.value (.bool true)` too, so `denote (.ite c t e) = denote t = denote (normalize t)` (`iht`). These are
the equality-form ite sub-cases the `denote.induct` capstone consumes for a fold-select condition. -/
theorem denote_normalize_ite_condTrue_eq (ρ : Nat → Value) (w : IntTy) (c t e : SymExpr)
    (hc : normalize c = .const (.bool true))
    (ihc : denote ρ w (normalize c) = denote ρ w c)
    (iht : denote ρ w (normalize t) = denote ρ w t) :
    denote ρ w (normalize (.ite c t e)) = denote ρ w (.ite c t e) := by
  have hdc : denote ρ w c = .value (.bool true) := by rw [← ihc, hc]; simp [denote, Value.asF64?]
  rw [normalize_ite_condTrue c t e hc, iht]
  simp only [denote, hdc]

theorem denote_normalize_ite_condFalse_eq (ρ : Nat → Value) (w : IntTy) (c t e : SymExpr)
    (hc : normalize c = .const (.bool false))
    (ihc : denote ρ w (normalize c) = denote ρ w c)
    (ihe : denote ρ w (normalize e) = denote ρ w e) :
    denote ρ w (normalize (.ite c t e)) = denote ρ w (.ite c t e) := by
  have hdc : denote ρ w c = .value (.bool false) := by rw [← ihc, hc]; simp [denote, Value.asF64?]
  rw [normalize_ite_condFalse c t e hc, ihe]
  simp only [denote, hdc]

/-- `normalize`-`ite` MATERIALIZE structural equations (now reducible after the ite arm was refactored
from `==` to a structural `match` on the branches). When both branches normalize to `true`/`false`, the
`ite` normalizes to the (normalized) condition; to `false`/`true`, to `not` of it. Holds for EVERY
condition shape (the const-fold arms return the folded literal = `normalize c`; the rebuild arm's first
`match` clause fires) — no `≠ const bool` hypothesis needed for the true case. -/
theorem normalize_ite_materializeTrue (c t e : SymExpr)
    (hnt : normalize t = .const (.bool true)) (hne : normalize e = .const (.bool false)) :
    normalize (.ite c t e) = normalize c := by
  simp only [normalize, hnt, hne]
  split <;> simp_all

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

/-! ### Fold-alignment lemmas: `denote`'s app-combiner agrees with `normalize`'s const-fold.
These are the reusable ENGINE the eventual capstone `.app` fold-subcase invokes. `normalize` folds a
binary app of two constants to `.const v` exactly when `foldConst? op #[.const a, .const b] = some v`
(comparison/boolean/float/equality) or, for the deferred integer arithmetic `foldConst?` leaves alone,
to the real `evalArithOp` result. Here we show `denote`'s combiner (`denoteBinary`/`denoteApp`) produces
that SAME outcome — so once a `normalize`-app equation lemma is available, the capstone `.app` case
reduces to these. They are value-conditioned in spirit (they speak only about `.value` operands, so the
ill-typed-`e` typing subtlety never arises) and gate green independently of that design point. -/
theorem denoteBinary_fold (op : String) (w : IntTy) (va vb v : Value)
    (h : foldConst? op #[.const va, .const vb] = some v) :
    denoteBinary op w (.value va) (.value vb) = .value v := by
  simp only [denoteBinary, h]

theorem denoteApp_fold (op : String) (w : IntTy) (va vb v : Value)
    (h : foldConst? op #[.const va, .const vb] = some v) :
    denoteApp op w #[.value va, .value vb] = .value v := by
  have hred : denoteApp op w #[.value va, .value vb]
      = denoteBinary op w (.value va) (.value vb) := rfl
  rw [hred]; exact denoteBinary_fold op w va vb v h

/-- UNARY fold-soundness (the arity-1 companion of `denoteBinary_fold`/`denoteApp_fold`, for `not`):
`denoteUnary` reuses `foldConst? op #[.const va]`, so when that folds to `v`, the combiner yields `.value v`. -/
theorem denoteUnary_fold (op : String) (va v : Value)
    (h : foldConst? op #[.const va] = some v) :
    denoteUnary op (.value va) = .value v := by
  simp only [denoteUnary, h]

theorem denoteApp_fold_unary (op : String) (w : IntTy) (va v : Value)
    (h : foldConst? op #[.const va] = some v) :
    denoteApp op w #[.value va] = .value v := by
  have hred : denoteApp op w #[.value va] = denoteUnary op (.value va) := rfl
  rw [hred]; exact denoteUnary_fold op va v h

/-! ### `denote`-app VALUE INVERSION: a `.value`-producing application had `.value` operands.
`denoteUnary`/`denoteBinary` return a `.value` ONLY through their leading `.value`(-`.value`) arm — every
other operand outcome (`.trap`/`.diverges`/`.unsupported`) is propagated verbatim (`| o => o`, the trap
arms) or falls to `.unsupported`. So a `.value` result WITNESSES that the operands were themselves values.
The capstone `.app` case needs this to turn `denote (.app op args) = .value v` into per-operand
`denote aᵢ = .value uᵢ` facts, which the argument IHs then transport across `normalize`. -/
theorem denoteUnary_value_inv (op : String) (oa : Outcome) (v : Value)
    (h : denoteUnary op oa = .value v) : ∃ u, oa = .value u := by
  cases oa with
  | value u => exact ⟨u, rfl⟩
  | _ => simp [denoteUnary] at h

theorem denoteBinary_value_inv (op : String) (w : IntTy) (oa ob : Outcome) (v : Value)
    (h : denoteBinary op w oa ob = .value v) : (∃ ua, oa = .value ua) ∧ (∃ ub, ob = .value ub) := by
  cases oa <;> cases ob <;>
    first
      | exact ⟨⟨_, rfl⟩, ⟨_, rfl⟩⟩
      | simp [denoteBinary] at h

/-- `denoteApp` VALUE INVERSION: a `.value` result forces arity 1 (a `.value` operand) or 2 (two `.value`
operands). Combines the arity dispatch with the unary/binary operand inversions above. -/
theorem denoteApp_value_inv (op : String) (w : IntTy) (oargs : Array Outcome) (v : Value)
    (h : denoteApp op w oargs = .value v) :
    (oargs.size = 1 ∧ ∃ u, oargs[0]! = .value u) ∨
    (oargs.size = 2 ∧ (∃ u0, oargs[0]! = .value u0) ∧ (∃ u1, oargs[1]! = .value u1)) := by
  unfold denoteApp at h
  split at h
  · rename_i h1
    left
    refine ⟨h1, ?_⟩
    have hu := denoteUnary_value_inv op _ v h
    rwa [getElem!_pos oargs 0 (by omega)]
  · split at h
    · rename_i h2
      right
      refine ⟨h2, ?_, ?_⟩
      · have hb := (denoteBinary_value_inv op w _ _ v h).1
        rwa [getElem!_pos oargs 0 (by omega)]
      · have hb := (denoteBinary_value_inv op w _ _ v h).2
        rwa [getElem!_pos oargs 1 (by omega)]
    · exact absurd h (by simp)

/-- The deferred integer-arithmetic path: when `foldConst?` declines (int `+ - * / %` are not folded —
their overflow-trap conditions are width-dependent) and the op is arithmetic, `denote`'s combiner falls
through to the REAL width-`w` `evalArithOp` (byte-identical to `evalNode`), so its trap/overflow
semantics are the oracle's, not a re-implementation. -/
theorem denoteBinary_arith (op : String) (w : IntTy) (x y : Int)
    (hf : foldConst? op #[.const (.int x), .const (.int y)] = none)
    (hop : arithOps.contains op = true) :
    denoteBinary op w (.value (.int x)) (.value (.int y)) = evalArithOp op x y w := by
  simp only [denoteBinary, hf, hop, if_true]

/-! ### Arith-operand INT inversion: an arithmetic `denoteBinary` yielding a `.value` had INT operands.
When `foldConst?` declines (given) and the op is arithmetic, `denoteBinary`'s deferred path is
`match va, vb with | .int x, .int y => evalArithOp … | _ => .unsupported` — so a `.value` result forces
BOTH operands to be `.int`. The capstone `.app`-IDENTITY sub-cases (`x+0→x`, `x*1→x`, …) need this to turn
a generic surviving operand into the `.int x` shape the `denoteBinary_*_value` lemmas require. -/
theorem denoteBinary_arith_int_r (op : String) (w : IntTy) (va : Value) (y : Int) (v : Value)
    (hf : foldConst? op #[.const va, .const (.int y)] = none) (hop : arithOps.contains op = true)
    (h : denoteBinary op w (.value va) (.value (.int y)) = .value v) : ∃ x, va = .int x := by
  cases va <;> first | exact ⟨_, rfl⟩ | simp_all [denoteBinary]

theorem denoteBinary_arith_int_l (op : String) (w : IntTy) (x : Int) (vb : Value) (v : Value)
    (hf : foldConst? op #[.const (.int x), .const vb] = none) (hop : arithOps.contains op = true)
    (h : denoteBinary op w (.value (.int x)) (.value vb) = .value v) : ∃ y, vb = .int y := by
  cases vb <;> first | exact ⟨_, rfl⟩ | simp_all [denoteBinary]

/-! ### Arithmetic-identity value characterizations: the constant-RESULT normalizer folds are denote-sound.
The operand-DROPPING algebraic identities (`x*0→0`, `0*x→0`, `x-x→0`, `x%1→0` — each `!mayTrap`-guarded)
rewrite to `.const 0`. Their denote-soundness is exactly: whenever the arithmetic op yields a value at
ALL, that value is `.int 0`. Stated in VALUE-CONDITIONED form (`… = .value v → v = .int 0`), so they are
unconditionally true — vacuous under an unresolved width (`evalArithOp` = `.unsupported`) or an
out-of-range/overflow trap (`.trap`), which sidesteps the width/typing subtlety. These are the concrete
arithmetic core the eventual capstone `.app`-identity subcase consumes (via `denoteBinary_arith`). -/
theorem evalArithOp_mul_zero_r (x : Int) (ty : IntTy) (v : Value)
    (h : evalArithOp "*" x 0 ty = .value v) : v = .int 0 := by
  simp only [evalArithOp, Int.mul_zero] at h
  split at h <;> simp_all
  all_goals (try (split at h <;> simp_all))
  all_goals (try (split at h <;> simp_all))
  all_goals (try (split at h <;> simp_all))

theorem evalArithOp_mul_zero_l (y : Int) (ty : IntTy) (v : Value)
    (h : evalArithOp "*" 0 y ty = .value v) : v = .int 0 := by
  simp only [evalArithOp, Int.zero_mul] at h
  split at h <;> simp_all
  all_goals (try (split at h <;> simp_all))
  all_goals (try (split at h <;> simp_all))
  all_goals (try (split at h <;> simp_all))

theorem evalArithOp_sub_self (x : Int) (ty : IntTy) (v : Value)
    (h : evalArithOp "-" x x ty = .value v) : v = .int 0 := by
  simp only [evalArithOp, Int.sub_self] at h
  split at h <;> simp_all
  all_goals (try (split at h <;> simp_all))
  all_goals (try (split at h <;> simp_all))
  all_goals (try (split at h <;> simp_all))

theorem evalArithOp_mod_one (x : Int) (ty : IntTy) (v : Value)
    (h : evalArithOp "%" x 1 ty = .value v) : v = .int 0 := by
  simp only [evalArithOp, Int.tmod_one] at h
  split at h <;> simp_all
  all_goals (try (split at h <;> simp_all))
  all_goals (try (split at h <;> simp_all))
  all_goals (try (split at h <;> simp_all))
  all_goals (try (split at h <;> simp_all))

/-! ### Arithmetic-identity value characterizations, operand-PRESERVING case.
The value-PRESERVING algebraic identities (`x+0→x`, `x-0→x`, `x*1→x`, `1*x→x`, `x/1→x`) rewrite to the
surviving operand. Their denote-soundness is: whenever the op yields a value, that value is the operand.
Same VALUE-CONDITIONED shape as the dropping case (vacuous under unresolved width / out-of-range trap),
so unconditionally true. Together with the dropping lemmas above and the `foldConst?` alignment
(`denoteBinary_fold`, #6405), these are the complete arithmetic engine for the capstone `.app`-identity
subcase: every arithmetic normalizer identity's result value is pinned. -/
theorem evalArithOp_add_zero_r (x : Int) (ty : IntTy) (v : Value)
    (h : evalArithOp "+" x 0 ty = .value v) : v = .int x := by
  simp only [evalArithOp, Int.add_zero] at h
  repeat' (first | (split at h) | simp_all (config := { decide := true }))

/-- Left additive identity `0 + y` (the `0 + x → x` normalizer arm's arithmetic core). -/
theorem evalArithOp_add_zero_l (y : Int) (ty : IntTy) (v : Value)
    (h : evalArithOp "+" 0 y ty = .value v) : v = .int y := by
  simp only [evalArithOp, Int.zero_add] at h
  repeat' (first | (split at h) | simp_all (config := { decide := true }))

theorem evalArithOp_sub_zero_r (x : Int) (ty : IntTy) (v : Value)
    (h : evalArithOp "-" x 0 ty = .value v) : v = .int x := by
  simp only [evalArithOp, Int.sub_zero] at h
  repeat' (first | (split at h) | simp_all (config := { decide := true }))

theorem evalArithOp_mul_one_r (x : Int) (ty : IntTy) (v : Value)
    (h : evalArithOp "*" x 1 ty = .value v) : v = .int x := by
  simp only [evalArithOp, Int.mul_one] at h
  repeat' (first | (split at h) | simp_all (config := { decide := true }))

theorem evalArithOp_mul_one_l (y : Int) (ty : IntTy) (v : Value)
    (h : evalArithOp "*" 1 y ty = .value v) : v = .int y := by
  simp only [evalArithOp, Int.one_mul] at h
  repeat' (first | (split at h) | simp_all (config := { decide := true }))

theorem evalArithOp_div_one (x : Int) (ty : IntTy) (v : Value)
    (h : evalArithOp "/" x 1 ty = .value v) : v = .int x := by
  simp only [evalArithOp, Int.tdiv_one] at h
  repeat' (first | (split at h) | simp_all (config := { decide := true }))

/-! ### `denoteBinary`-level arithmetic-identity value characterizations (the capstone `.app` bridge).
Composing `denoteBinary_arith` (the deferred int-arith path routes to `evalArithOp`, given the op is
arithmetic and `foldConst?` declined, #6405) with the `evalArithOp` characterizations above pins the
result value of `denoteBinary` DIRECTLY, in the exact shape the capstone `.app` case consumes. The
`foldConst? … = none` hypothesis is precisely what that case has IN HAND on the `normalize`-identity
branch (`normalize`'s `.app` arm applies the algebraic identities only when `foldConst?` returned
`none`), so carrying it here is faithful, not a gap. Two representatives — an operand-dropping (`x*0`)
and an operand-preserving (`x+0`) one; the remaining identities follow by the identical composition. -/
theorem denoteBinary_mul_zero_value (w : IntTy) (x : Int) (v : Value)
    (hf : foldConst? "*" #[.const (.int x), .const (.int 0)] = none)
    (h : denoteBinary "*" w (.value (.int x)) (.value (.int 0)) = .value v) : v = .int 0 :=
  evalArithOp_mul_zero_r x w v (denoteBinary_arith "*" w x 0 hf (by decide) ▸ h)

theorem denoteBinary_add_zero_value (w : IntTy) (x : Int) (v : Value)
    (hf : foldConst? "+" #[.const (.int x), .const (.int 0)] = none)
    (h : denoteBinary "+" w (.value (.int x)) (.value (.int 0)) = .value v) : v = .int x :=
  evalArithOp_add_zero_r x w v (denoteBinary_arith "+" w x 0 hf (by decide) ▸ h)

theorem denoteBinary_add_zero_l_value (w : IntTy) (y : Int) (v : Value)
    (hf : foldConst? "+" #[.const (.int 0), .const (.int y)] = none)
    (h : denoteBinary "+" w (.value (.int 0)) (.value (.int y)) = .value v) : v = .int y :=
  evalArithOp_add_zero_l y w v (denoteBinary_arith "+" w 0 y hf (by decide) ▸ h)

-- The remaining operand-DROPPING identities (`0*x→0`, `x-x→0`, `x%1→0`).
theorem denoteBinary_mul_zero_l_value (w : IntTy) (y : Int) (v : Value)
    (hf : foldConst? "*" #[.const (.int 0), .const (.int y)] = none)
    (h : denoteBinary "*" w (.value (.int 0)) (.value (.int y)) = .value v) : v = .int 0 :=
  evalArithOp_mul_zero_l y w v (denoteBinary_arith "*" w 0 y hf (by decide) ▸ h)

theorem denoteBinary_sub_self_value (w : IntTy) (x : Int) (v : Value)
    (hf : foldConst? "-" #[.const (.int x), .const (.int x)] = none)
    (h : denoteBinary "-" w (.value (.int x)) (.value (.int x)) = .value v) : v = .int 0 :=
  evalArithOp_sub_self x w v (denoteBinary_arith "-" w x x hf (by decide) ▸ h)

theorem denoteBinary_mod_one_value (w : IntTy) (x : Int) (v : Value)
    (hf : foldConst? "%" #[.const (.int x), .const (.int 1)] = none)
    (h : denoteBinary "%" w (.value (.int x)) (.value (.int 1)) = .value v) : v = .int 0 :=
  evalArithOp_mod_one x w v (denoteBinary_arith "%" w x 1 hf (by decide) ▸ h)

-- The remaining operand-PRESERVING identities (`x-0→x`, `x*1→x`, `1*x→x`, `x/1→x`).
theorem denoteBinary_sub_zero_value (w : IntTy) (x : Int) (v : Value)
    (hf : foldConst? "-" #[.const (.int x), .const (.int 0)] = none)
    (h : denoteBinary "-" w (.value (.int x)) (.value (.int 0)) = .value v) : v = .int x :=
  evalArithOp_sub_zero_r x w v (denoteBinary_arith "-" w x 0 hf (by decide) ▸ h)

theorem denoteBinary_mul_one_r_value (w : IntTy) (x : Int) (v : Value)
    (hf : foldConst? "*" #[.const (.int x), .const (.int 1)] = none)
    (h : denoteBinary "*" w (.value (.int x)) (.value (.int 1)) = .value v) : v = .int x :=
  evalArithOp_mul_one_r x w v (denoteBinary_arith "*" w x 1 hf (by decide) ▸ h)

theorem denoteBinary_mul_one_l_value (w : IntTy) (y : Int) (v : Value)
    (hf : foldConst? "*" #[.const (.int 1), .const (.int y)] = none)
    (h : denoteBinary "*" w (.value (.int 1)) (.value (.int y)) = .value v) : v = .int y :=
  evalArithOp_mul_one_l y w v (denoteBinary_arith "*" w 1 y hf (by decide) ▸ h)

theorem denoteBinary_div_one_value (w : IntTy) (x : Int) (v : Value)
    (hf : foldConst? "/" #[.const (.int x), .const (.int 1)] = none)
    (h : denoteBinary "/" w (.value (.int x)) (.value (.int 1)) = .value v) : v = .int x :=
  evalArithOp_div_one x w v (denoteBinary_arith "/" w x 1 hf (by decide) ▸ h)

/-! ### Capstone base cases: `denote (normalize e) = denote e` on the leaves.
The normalizer preserves meaning on `var` (it is the identity) and `const` (float canonicalization is
now aligned in `denote`, so the equality is structural). These are the base cases of the full
`denote (normalize e) = denote e` soundness theorem; its inductive cases — the `.app` fold/identities,
the `.ite` fold/collapse (which uses `mayTrap_sound`), and the compound-congruence cases — remain. -/
theorem denote_normalize_var (ρ : Nat → Value) (w : IntTy) (n : Nat) :
    denote ρ w (normalize (.var n)) = denote ρ w (.var n) := by rw [normalize_var]

theorem denote_normalize_const (ρ : Nat → Value) (w : IntTy) (v : Value) :
    denote ρ w (normalize (.const v)) = denote ρ w (.const v) := by
  have hf : ∀ f : Float, Value.asF64? (.f64 f) = some f := fun _ => rfl
  simp only [normalize, denote]
  cases h : Value.asF64? v <;> simp [h, hf]

/-- Float canonicalization (`asF64? v` → `.f64`, else `v` — the shared `.const`/`normalize`/`denote` canon)
is IDEMPOTENT. A float value maps to its `.f64` (whose `asF64?` is itself), a non-float is unchanged — so
re-canonicalizing is a no-op. This is the fact that a `normalize`-`const` output (already canon'd via the
`.const` arm) is canon-STABLE, which the capstone `.app`/`.const` cases rely on when relating a folded
constant to its denotation. -/
theorem asF64Canon_idem (v : Value) :
    (match Value.asF64? (match Value.asF64? v with | some f => Value.f64 f | none => v) with
     | some g => Value.f64 g | none => (match Value.asF64? v with | some f => Value.f64 f | none => v))
    = (match Value.asF64? v with | some f => Value.f64 f | none => v) := by
  have hf : ∀ f : Float, Value.asF64? (.f64 f) = some f := fun _ => rfl
  cases h : Value.asF64? v <;> simp [h, hf]

/-- `evalFloatOp` yields ONLY `.f64` values: its `+`/`-`/`*`/`/` arms return `.value (.f64 …)` and `%` is
`.unsupported`, so whenever it produces a `.value`, that value is a `.f64`. A building block the capstone
`.app` fold case uses: the sole `foldConst?` FLOAT output flows through here, so it is canon-stable. -/
theorem evalFloatOp_value_f64 (op : String) (a b : Float) (w : Value)
    (h : evalFloatOp op a b = .value w) : ∃ g, w = Value.f64 g := by
  unfold evalFloatOp at h
  by_cases h1 : (op == "+") = true
  · rw [if_pos h1] at h; injection h with h'; exact ⟨_, h'.symm⟩
  rw [if_neg h1] at h
  by_cases h2 : (op == "-") = true
  · rw [if_pos h2] at h; injection h with h'; exact ⟨_, h'.symm⟩
  rw [if_neg h2] at h
  by_cases h3 : (op == "*") = true
  · rw [if_pos h3] at h; injection h with h'; exact ⟨_, h'.symm⟩
  rw [if_neg h3] at h
  by_cases h4 : (op == "/") = true
  · rw [if_pos h4] at h; injection h with h'; exact ⟨_, h'.symm⟩
  rw [if_neg h4] at h
  exact absurd h (by simp)

/-- The `foldConst?` FLOAT arm packages `evalFloatOp op x y` as `some w` on a `.value` outcome, `none`
otherwise. So whenever it yields `some v`, `v` came through `evalFloatOp` → is a `.f64` (via
`evalFloatOp_value_f64`). Building block for `foldConst?_out`'s single non-`.bool` arm. -/
private theorem evalFloatOp_fold_f64 (op : String) (x y : Float) (v : Value)
    (h : (match evalFloatOp op x y with | .value w => some w | _ => none) = some v) :
    ∃ g, v = Value.f64 g := by
  split at h
  · injection h with h'
    obtain ⟨g, hg⟩ := evalFloatOp_value_f64 op x y _ (by assumption)
    exact ⟨g, h' ▸ hg⟩
  · simp at h

/-- Every `foldConst?` output is a `.bool` (all comparison / `=` / `and` / `or` / `not` arms) or a `.f64`
(the sole FLOAT-arith arm, via `evalFloatOp`) — NEVER an `.int`/`.float`/`.floatNan`/`.floatInf`. This is
the fact the capstone `.app` fold case needs: a folded constant is `asF64?`-canonical (`foldConst?_canon_stable`
below). Proven by MANUAL `by_cases` on each `if op == …` condition (core `split` does not fire on the
`if (bool) = true` chains, and `split_ifs` is Mathlib-only — see the tactic note in #6997), collapsing via
`rw [if_pos/if_neg]`; the inner `Value`/`Option`/`asF64?` matches DO `split`. -/
theorem foldConst?_out (op : String) (args : Array SymExpr) (v : Value)
    (h : foldConst? op args = some v) :
    (∃ b, v = Value.bool b) ∨ (∃ f, v = Value.f64 f) := by
  unfold foldConst? at h
  -- `consts.any isNone` guard
  by_cases hn : (args.map symToValue?).any (·.isNone) = true
  · rw [if_pos hn] at h; simp at h
  rw [if_neg hn] at h
  -- unary `not` size-dispatch
  by_cases h1 : (op == "not" && ((args.map symToValue?).filterMap id).size == 1) = true
  · rw [if_pos h1] at h
    split at h
    · injection h with h'; exact Or.inl ⟨_, h'.symm⟩
    · simp at h
  rw [if_neg h1] at h
  -- binary size-2 dispatch
  by_cases h2 : (((args.map symToValue?).filterMap id).size == 2) = true
  · rw [if_pos h2] at h
    by_cases he : (op == "=") = true
    · rw [if_pos he] at h; injection h with h'; exact Or.inl ⟨_, h'.symm⟩
    rw [if_neg he] at h
    by_cases hlt : (op == "<") = true
    · rw [if_pos hlt] at h
      split at h
      · injection h with h'; exact Or.inl ⟨_, h'.symm⟩
      · split at h
        · injection h with h'; exact Or.inl ⟨_, h'.symm⟩
        · split at h
          · injection h with h'; exact Or.inl ⟨_, h'.symm⟩
          · simp at h
    rw [if_neg hlt] at h
    by_cases hgt : (op == ">") = true
    · rw [if_pos hgt] at h
      split at h
      · injection h with h'; exact Or.inl ⟨_, h'.symm⟩
      · split at h
        · injection h with h'; exact Or.inl ⟨_, h'.symm⟩
        · split at h
          · injection h with h'; exact Or.inl ⟨_, h'.symm⟩
          · simp at h
    rw [if_neg hgt] at h
    by_cases hle : (op == "<=") = true
    · rw [if_pos hle] at h
      split at h
      · injection h with h'; exact Or.inl ⟨_, h'.symm⟩
      · split at h
        · injection h with h'; exact Or.inl ⟨_, h'.symm⟩
        · split at h
          · injection h with h'; exact Or.inl ⟨_, h'.symm⟩
          · simp at h
    rw [if_neg hle] at h
    by_cases hge : (op == ">=") = true
    · rw [if_pos hge] at h
      split at h
      · injection h with h'; exact Or.inl ⟨_, h'.symm⟩
      · split at h
        · injection h with h'; exact Or.inl ⟨_, h'.symm⟩
        · split at h
          · injection h with h'; exact Or.inl ⟨_, h'.symm⟩
          · simp at h
    rw [if_neg hge] at h
    by_cases har : (op == "+" || op == "-" || op == "*" || op == "/") = true
    · rw [if_pos har] at h
      split at h
      · exact Or.inr (evalFloatOp_fold_f64 op _ _ v h)
      · simp at h
    rw [if_neg har] at h
    by_cases hand : (op == "and") = true
    · rw [if_pos hand] at h
      split at h
      · injection h with h'; exact Or.inl ⟨_, h'.symm⟩
      · split at h
        · injection h with h'; exact Or.inl ⟨_, h'.symm⟩
        · simp at h
    rw [if_neg hand] at h
    by_cases hor : (op == "or") = true
    · rw [if_pos hor] at h
      split at h
      · injection h with h'; exact Or.inl ⟨_, h'.symm⟩
      · split at h
        · injection h with h'; exact Or.inl ⟨_, h'.symm⟩
        · simp at h
    rw [if_neg hor] at h
    simp at h
  rw [if_neg h2] at h
  -- variadic (arity ≠ 2) `and`/`or`
  by_cases hand2 : (op == "and") = true
  · rw [if_pos hand2] at h
    split at h
    · injection h with h'; exact Or.inl ⟨_, h'.symm⟩
    · split at h
      · injection h with h'; exact Or.inl ⟨_, h'.symm⟩
      · simp at h
  rw [if_neg hand2] at h
  by_cases hor2 : (op == "or") = true
  · rw [if_pos hor2] at h
    split at h
    · injection h with h'; exact Or.inl ⟨_, h'.symm⟩
    · split at h
      · injection h with h'; exact Or.inl ⟨_, h'.symm⟩
      · simp at h
  rw [if_neg hor2] at h
  simp at h

/-- CAPSTONE `.app` fold-step: a `foldConst?` output is `asF64?`-CANON-STABLE — canonicalizing it (the shared
`.const`/`normalize`/`denote` float canon) is a no-op. Immediate from `foldConst?_out` (every output is `.bool`
— `asF64? = none` → canon returns it unchanged — or `.f64 f` — `asF64? = some f` → canon returns `.f64 f`
unchanged). This is what lets the capstone `.app` fold case relate a folded constant to its denotation without
a further canonicalization step. -/
theorem foldConst?_canon_stable (op : String) (args : Array SymExpr) (v : Value)
    (h : foldConst? op args = some v) :
    (match Value.asF64? v with | some f => Value.f64 f | none => v) = v := by
  rcases foldConst?_out op args v h with ⟨b, rfl⟩ | ⟨f, rfl⟩ <;> rfl

/-- `foldConst?` reads its operands ONLY through `args.map symToValue?` — so two arg arrays with the same
`symToValue?` image fold identically. This lets the capstone `.app` fold case swap a NORMALIZED operand for
`.const (its symToValue? value)` (they have the same image) to reach `denoteBinary_fold`/`denoteUnary_fold`. -/
theorem foldConst?_symToValue_congr (op : String) (args1 args2 : Array SymExpr)
    (h : args1.map symToValue? = args2.map symToValue?) :
    foldConst? op args1 = foldConst? op args2 := by
  simp only [foldConst?, h]

/-- A binary `foldConst?` that FIRES has BOTH operands `symToValue?`-extractable (its `consts.any isNone`
guard is false). The 2-operand form the capstone `.app` fold case needs (to invoke `denote_symToValue` on
each operand). Proven by casing the two `symToValue?`s on the concrete 2-array (the guard reduces). -/
theorem foldConst?_binary_operands_some (op : String) (x0 x1 : SymExpr) (v : Value)
    (h : foldConst? op #[x0, x1] = some v) :
    (∃ c0, symToValue? x0 = some c0) ∧ (∃ c1, symToValue? x1 = some c1) := by
  cases h0 : symToValue? x0 with
  | none => rw [foldConst?] at h; simp [h0] at h
  | some c0 =>
    cases h1 : symToValue? x1 with
    | none => rw [foldConst?] at h; simp [h0, h1] at h
    | some c1 => exact ⟨⟨c0, rfl⟩, ⟨c1, rfl⟩⟩

/-- Unary companion: a `not`-fold that fires has its single operand `symToValue?`-extractable. -/
theorem foldConst?_unary_operand_some (op : String) (x0 : SymExpr) (v : Value)
    (h : foldConst? op #[x0] = some v) : ∃ c0, symToValue? x0 = some c0 := by
  cases h0 : symToValue? x0 with
  | none => rw [foldConst?] at h; simp [h0] at h
  | some c0 => exact ⟨c0, rfl⟩

/-- `foldConst?` DECLINES (`= none`) as soon as its FIRST operand is not `symToValue?`-extractable — its
`consts.any (·.isNone)` guard short-circuits on the leading `none`. This is the exact `foldConst? … = none`
hypothesis the capstone `.app`-IDENTITY branch has in hand: `normalize`'s `.app` arm applies an algebraic
identity (`x+0→x`, `x*1→x`, …) only after `foldConst?` returned `none`, and it returns `none` here precisely
because the surviving operand is symbolic (a `var`/`app`, so `symToValue? = none`). Companion of
`foldConst?_binary_operands_some` (the fires-⇒-both-some direction). -/
theorem foldConst?_none_of_fst_symbolic (op : String) (a b : SymExpr)
    (ha : symToValue? a = none) : foldConst? op #[a, b] = none := by
  rw [foldConst?]; simp [ha]

/-! ### `.app`-IDENTITY capstone bridge — UNBLOCKED (2026-08-31).
The `.app`-identity soundness cases were blocked because `normalizeAppIdentities`'s const-identity guards
fired on `e == .const (.int n)`, whose `SymExpr` derived `BEq` is `opaque` (kernel-irreducible — only
`native_decide` reduces it, which is axiom-dirty). Fixed at the source: those guards now use the REDUCIBLE
structural checker `isConstInt` (Symbolic.lean), byte-identical to the `==` form. So (a) the syntactic
identity-EQUATION lemmas below now REDUCE, and (b) `isConstInt_eq` supplies the `e = .const (.int n)`
fact — the LawfulBEq-on-int-consts fragment the opaque `==` could never give. -/

/-- `isConstInt e n = true` ⇒ `e` is SYNTACTICALLY the literal `.const (.int n)` (structural, reducible). -/
theorem isConstInt_eq (e : SymExpr) (n : Int) (h : isConstInt e n = true) : e = .const (.int n) := by
  unfold isConstInt at h; split at h
  · rename_i m; simp only [beq_iff_eq] at h; subst h; rfl
  · exact absurd h (by simp)

/-- Bool companion: `isConstBool e b = true` ⇒ `e = .const (.bool b)`. -/
theorem isConstBool_eq (e : SymExpr) (b : Bool) (h : isConstBool e b = true) : e = .const (.bool b) := by
  unfold isConstBool at h; split at h
  · rename_i c; simp only [beq_iff_eq] at h; subst h; rfl
  · exact absurd h (by simp)

-- The operand-PRESERVING const-identity EQUATION lemmas: `normalizeAppIdentities` returns the surviving
-- operand unchanged on these shapes (no `!mayTrap` guard — they preserve the operand incl. its traps). Each
-- reduces now that the guards are `isConstInt` (was structurally impossible under the opaque `==`). These
-- compose with the `denoteBinary_{add_zero,sub_zero,mul_one,div_one}_value` characterizations (@ ~600-650)
-- for the capstone `.app`-identity denote-soundness assembly. (Operand-DROPPING `x*0`/`x%1`/`x&0`/`x^x` are
-- conditional on `!mayTrap` → a separate follow-up.)
theorem normalizeAppIdentities_add_zero_r (a : SymExpr) :
    normalizeAppIdentities "+" #[a, .const (.int 0)] = a := by simp [normalizeAppIdentities, isConstInt]
theorem normalizeAppIdentities_add_zero_l (b : SymExpr) (hb : isConstInt b 0 = false) :
    normalizeAppIdentities "+" #[.const (.int 0), b] = b := by
  have h2 : isConstInt (.const (.int 0)) 0 = true := rfl
  simp [normalizeAppIdentities, hb, h2]
theorem normalizeAppIdentities_sub_zero_r (a : SymExpr) :
    normalizeAppIdentities "-" #[a, .const (.int 0)] = a := by simp [normalizeAppIdentities, isConstInt]
theorem normalizeAppIdentities_mul_one_r (a : SymExpr) :
    normalizeAppIdentities "*" #[a, .const (.int 1)] = a := by simp [normalizeAppIdentities, isConstInt]
theorem normalizeAppIdentities_mul_one_l (b : SymExpr) (hb : isConstInt b 1 = false) :
    normalizeAppIdentities "*" #[.const (.int 1), b] = b := by
  have h2 : isConstInt (.const (.int 1)) 1 = true := rfl
  simp [normalizeAppIdentities, hb, h2]
theorem normalizeAppIdentities_div_one_r (a : SymExpr) :
    normalizeAppIdentities "/" #[a, .const (.int 1)] = a := by simp [normalizeAppIdentities, isConstInt]

/-- Every `.const` LEAF anywhere in a symbolic expression is `asF64?`-canonical (its float components are
already `.f64`, never a `.float`/`.floatNan`/`.floatInf` spelling). This is the invariant `normalize`
OUTPUTS satisfy (`normalize_allConstsCanon`): the `.const` arm canonicalizes float literals to `.f64`, and
every folded constant is canon-stable (`foldConst?_canon_stable`). Stated over ALL const leaves (not just
the root) so it survives the `not (not x) → x` rewrite, which returns a GRANDCHILD of a normalized arg. A
`.localFn` capture list is ignored (a `localFn` is never a fold operand — `symToValue?` rejects it). -/
def AllConstsCanon : SymExpr → Prop
  | .const c => (match Value.asF64? c with | some f => Value.f64 f | none => c) = c
  | .var _ => True
  | .app _ args => ∀ x ∈ args, AllConstsCanon x
  | .ite c t e => AllConstsCanon c ∧ AllConstsCanon t ∧ AllConstsCanon e
  | .tuple es => ∀ x ∈ es, AllConstsCanon x
  | .record fs => ∀ k e, (k, e) ∈ fs → AllConstsCanon e
  | .ctor _ args => ∀ x ∈ args, AllConstsCanon x
  | .proj b _ => AllConstsCanon b
  | .case s arms => AllConstsCanon s ∧ ∀ k e, (k, e) ∈ arms → AllConstsCanon e
  | .localFn _ _ _ => True
termination_by e => sizeOf e
decreasing_by
  all_goals simp_wf
  all_goals
    first
      | omega
      | (have h := Array.sizeOf_lt_of_mem ‹_›; omega)
      | (rename_i hmem; have h := Array.sizeOf_lt_of_mem hmem; simp_all; omega)

/-- A literal int/bool constant is trivially `asF64?`-canonical (`asF64?` is `none` on it). -/
theorem allConstsCanon_int (n : Int) : AllConstsCanon (.const (.int n)) := by
  simp only [AllConstsCanon, Value.asF64?]
theorem allConstsCanon_bool (b : Bool) : AllConstsCanon (.const (.bool b)) := by
  simp only [AllConstsCanon, Value.asF64?]

/-- `normalizeAppIdentities` PRESERVES `AllConstsCanon`: if every operand's const leaves are canonical, so
are the result's. Every arm returns an OPERAND (`a`/`b`/the double-neg grandchild `inner`), a literal
`.const (.int 0)`/`.const (.bool _)`, or the rebuilt `.app op args'` — all canon-canonical given `h`.
Provable now that the def is a REDUCIBLE size-dispatch + `if op == …` chain (#7028): `split` cases the
goal-position `if` chains, and each leaf closes by the operand/literal facts. This discharges the `.app`
IDENTITY sub-case of `normalize_allConstsCanon`. -/
theorem normalizeAppIdentities_allConstsCanon (op : String) (args' : Array SymExpr)
    (h : ∀ x ∈ args', AllConstsCanon x) :
    AllConstsCanon (normalizeAppIdentities op args') := by
  have happ : AllConstsCanon (SymExpr.app op args') := by simp only [AllConstsCanon]; exact h
  have mem0 : 0 < args'.size → args'[0]! ∈ args' := fun h0 => by
    rw [getElem!_pos args' 0 h0]; exact args'.getElem_mem h0
  have mem1 : 1 < args'.size → args'[1]! ∈ args' := fun h1 => by
    rw [getElem!_pos args' 1 h1]; exact args'.getElem_mem h1
  unfold normalizeAppIdentities
  by_cases hnot : (op == "not" && args'.size == 1) = true
  · rw [if_pos hnot]
    have h0 : 0 < args'.size := by
      simp only [Bool.and_eq_true, beq_iff_eq] at hnot; omega
    have hA0 : AllConstsCanon args'[0]! := h _ (mem0 h0)
    split
    · -- args'[0]! = .app o oa ; result = if o=="not" && oa.size==1 then oa[0]! else .app op args'
      rename_i o oa heq
      rw [heq] at hA0
      simp only [AllConstsCanon] at hA0
      split
      · rename_i hcond
        have hoa0 : 0 < oa.size := by
          simp only [Bool.and_eq_true, beq_iff_eq] at hcond; omega
        rw [getElem!_pos oa 0 hoa0]
        exact hA0 _ (oa.getElem_mem hoa0)
      · exact happ
    · exact happ
  rw [if_neg hnot]
  by_cases hsz : (args'.size == 2) = true
  · rw [if_pos hsz]
    have hs2 : args'.size = 2 := by simpa using hsz
    have hAa : AllConstsCanon args'[0]! := h _ (mem0 (by omega))
    have hAb : AllConstsCanon args'[1]! := h _ (mem1 (by omega))
    dsimp only
    -- Every op arm's leaves are `a`/`b`/a literal const/`.app op args'`; `split` cases the (SymExpr-==)
    -- inner `if`s and each leaf closes. `by_cases` on the op string (core `split` won't case a String-== if).
    by_cases h1 : (op == "+") = true
    · rw [if_pos h1]
      repeat' (first | exact hAa | exact hAb | exact happ | exact allConstsCanon_int 0 | exact allConstsCanon_bool true | exact allConstsCanon_bool false | split)
    rw [if_neg h1]
    by_cases h2 : (op == "-") = true
    · rw [if_pos h2]
      repeat' (first | exact hAa | exact hAb | exact happ | exact allConstsCanon_int 0 | exact allConstsCanon_bool true | exact allConstsCanon_bool false | split)
    rw [if_neg h2]
    by_cases h3 : (op == "*") = true
    · rw [if_pos h3]
      repeat' (first | exact hAa | exact hAb | exact happ | exact allConstsCanon_int 0 | exact allConstsCanon_bool true | exact allConstsCanon_bool false | split)
    rw [if_neg h3]
    by_cases h4 : (op == "/") = true
    · rw [if_pos h4]
      repeat' (first | exact hAa | exact hAb | exact happ | exact allConstsCanon_int 0 | exact allConstsCanon_bool true | exact allConstsCanon_bool false | split)
    rw [if_neg h4]
    by_cases h5 : (op == "%") = true
    · rw [if_pos h5]
      repeat' (first | exact hAa | exact hAb | exact happ | exact allConstsCanon_int 0 | exact allConstsCanon_bool true | exact allConstsCanon_bool false | split)
    rw [if_neg h5]
    by_cases h6 : (op == "or") = true
    · rw [if_pos h6]
      repeat' (first | exact hAa | exact hAb | exact happ | exact allConstsCanon_int 0 | exact allConstsCanon_bool true | exact allConstsCanon_bool false | split)
    rw [if_neg h6]
    by_cases h7 : (op == "and") = true
    · rw [if_pos h7]
      repeat' (first | exact hAa | exact hAb | exact happ | exact allConstsCanon_int 0 | exact allConstsCanon_bool true | exact allConstsCanon_bool false | split)
    rw [if_neg h7]
    by_cases h8 : (op == "&") = true
    · rw [if_pos h8]
      repeat' (first | exact hAa | exact hAb | exact happ | exact allConstsCanon_int 0 | exact allConstsCanon_bool true | exact allConstsCanon_bool false | split)
    rw [if_neg h8]
    by_cases h9 : (op == "|") = true
    · rw [if_pos h9]
      repeat' (first | exact hAa | exact hAb | exact happ | exact allConstsCanon_int 0 | exact allConstsCanon_bool true | exact allConstsCanon_bool false | split)
    rw [if_neg h9]
    by_cases h10 : (op == "^") = true
    · rw [if_pos h10]
      repeat' (first | exact hAa | exact hAb | exact happ | exact allConstsCanon_int 0 | exact allConstsCanon_bool true | exact allConstsCanon_bool false | split)
    rw [if_neg h10]
    by_cases h11 : (op == "<<") = true
    · rw [if_pos h11]
      repeat' (first | exact hAa | exact hAb | exact happ | exact allConstsCanon_int 0 | exact allConstsCanon_bool true | exact allConstsCanon_bool false | split)
    rw [if_neg h11]
    by_cases h12 : (op == ">>") = true
    · rw [if_pos h12]
      repeat' (first | exact hAa | exact hAb | exact happ | exact allConstsCanon_int 0 | exact allConstsCanon_bool true | exact allConstsCanon_bool false | split)
    rw [if_neg h12]
    exact happ
  rw [if_neg hsz]
  exact happ

/-- `normalize` on an `.ite` PRESERVES `AllConstsCanon`, from the three sub-IHs. Every `.ite` sub-case
(fold-select / materialize-true|false / equal-branch collapse / plain rebuild) builds the result ONLY
from `normalize c`/`normalize t`/`normalize e` (± a `.app "not"` wrapper / a `.ite` rebuild), so its const
leaves come from theirs. Handles ALL six `normalize.induct` `.ite` cases uniformly. -/
theorem normalize_ite_allConstsCanon (c t e : SymExpr)
    (ihc : AllConstsCanon (normalize c)) (iht : AllConstsCanon (normalize t))
    (ihe : AllConstsCanon (normalize e)) :
    AllConstsCanon (normalize (.ite c t e)) := by
  simp only [normalize]
  repeat' first
    | exact ihc | exact iht | exact ihe
    | exact ⟨ihc, iht, ihe⟩
    | (intro x hx; simp only [Array.mem_singleton] at hx; subst hx; exact ihc)
    | (simp only [AllConstsCanon])
    | split

/-- CAPSTONE: every `.const` produced ANYWHERE by `normalize` is `asF64?`-canonical. Proven by
`normalize.induct`: the `.const` arm canonicalizes floats (`asF64Canon_idem`); a folded `.app` is a
`foldConst?` output (`foldConst?_canon_stable`); the identity `.app` preserves it
(`normalizeAppIdentities_allConstsCanon`); compounds recurse via the per-element IHs; `.ite` via
`normalize_ite_allConstsCanon`. This is the operand-canon-invariance the `.app`-fold assembly needs. -/
theorem normalize_allConstsCanon (e : SymExpr) : AllConstsCanon (normalize e) := by
  refine normalize.induct (motive := fun e => AllConstsCanon (normalize e))
    ?var ?const ?appFold ?appIdent ?tuple ?record ?ctor ?proj ?case ?localFn
    ?iteCT ?iteCF ?iteMT ?iteMF ?iteCol ?itePlain e
  case var => intro n; simp only [normalize, AllConstsCanon]
  case const => intro v; simp only [normalize, AllConstsCanon]; exact asF64Canon_idem v
  case appFold =>
    intro op args _ v hfold _
    rw [normalize_app_fold op args v hfold]
    simp only [AllConstsCanon]
    exact foldConst?_canon_stable _ _ _ hfold
  case appIdent =>
    intro op args _ hnone ih
    rw [normalize_app_ident op args hnone]
    apply normalizeAppIdentities_allConstsCanon
    intro x hx
    simp only [Array.mem_map] at hx
    obtain ⟨y, _, rfl⟩ := hx
    exact ih y
  case tuple =>
    intro es ih
    simp only [normalize, AllConstsCanon]
    intro x hx
    simp only [Array.mem_map] at hx
    obtain ⟨y, _, rfl⟩ := hx
    exact ih y
  case record =>
    intro fs ih
    simp only [normalize, AllConstsCanon]
    intro k v hmem
    simp only [Array.mem_map] at hmem
    obtain ⟨y, _, heq⟩ := hmem
    simp only [Prod.mk.injEq] at heq
    obtain ⟨_, rfl⟩ := heq
    exact ih y
  case ctor =>
    intro tag args ih
    simp only [normalize, AllConstsCanon]
    intro x hx
    simp only [Array.mem_map] at hx
    obtain ⟨y, _, rfl⟩ := hx
    exact ih y
  case proj => intro b s ih; simp only [normalize, AllConstsCanon]; exact ih
  case case =>
    intro s arms ihs iharms
    simp only [normalize, AllConstsCanon]
    refine ⟨ihs, ?_⟩
    intro k v hmem
    simp only [Array.mem_map] at hmem
    obtain ⟨y, _, heq⟩ := hmem
    simp only [Prod.mk.injEq] at heq
    obtain ⟨_, rfl⟩ := heq
    exact iharms y
  case localFn => intro s b c; simp only [normalize, AllConstsCanon]
  case iteCT => intro c t e hc ihc iht; rw [normalize_ite_condTrue c t e hc]; exact iht
  case iteCF => intro c t e hc ihc ihe; rw [normalize_ite_condFalse c t e hc]; exact ihe
  case iteMT => intro c t e _ _ _ _ _ _ ihc iht ihe; exact normalize_ite_allConstsCanon c t e ihc iht ihe
  case iteMF => intro c t e _ _ _ _ _ _ ihc iht ihe; exact normalize_ite_allConstsCanon c t e ihc iht ihe
  case iteCol => intro c t e _ _ _ _ _ _ _ ihc iht ihe; exact normalize_ite_allConstsCanon c t e ihc iht ihe
  case itePlain => intro c t e _ _ _ _ _ _ _ ihc iht ihe; exact normalize_ite_allConstsCanon c t e ihc iht ihe

/-- COROLLARY (the capstone `.app`-fold operand fact): a constant that `normalize` produces is
`asF64?`-canonical, so it needs no further canonicalization when related to its denotation. -/
theorem normalize_const_canon (e : SymExpr) (c : Value) (h : normalize e = .const c) :
    (match Value.asF64? c with | some f => Value.f64 f | none => c) = c := by
  have hac := normalize_allConstsCanon e
  rw [h] at hac
  simpa only [AllConstsCanon] using hac

/-! ### Capstone `.app`-fold SOUNDNESS CORE (scalar/const-operand case). When `normalize` folds an
application to a constant `vf` (its operands normalize to CONSTANTS and `foldConst?` fires), the ORIGINAL
application `denote`s to that SAME `vf`. These are stated at the `denoteBinary`/`denoteUnary` level (the
`.app` case reduces `denote (.app …)` to these via `denoteApp`); they carry the operand-value inversion +
the argument IHs + `normalize_const_canon` (a normalized constant is `asF64?`-canonical, so the folded
operand value EQUALS the denoted operand value) into `denoteBinary_fold`/`denoteUnary_fold`. This is the
heart of the capstone `.app` fold branch; the general (tuple/record-operand) fold needs the additional
`symToValue?`↔`denote` bridge (a later increment). -/
theorem denoteApp_normalize_fold_binary (ρ : Nat → Value) (w : IntTy) (op : String)
    (a0 a1 : SymExpr) (v c0 c1 vf : Value)
    (hn0 : normalize a0 = .const c0) (hn1 : normalize a1 = .const c1)
    (hfold : foldConst? op #[.const c0, .const c1] = some vf)
    (ih0 : ∀ u, denote ρ w a0 = .value u → denote ρ w (normalize a0) = .value u)
    (ih1 : ∀ u, denote ρ w a1 = .value u → denote ρ w (normalize a1) = .value u)
    (h : denoteBinary op w (denote ρ w a0) (denote ρ w a1) = .value v) : v = vf := by
  obtain ⟨⟨u0, hu0⟩, ⟨u1, hu1⟩⟩ := denoteBinary_value_inv op w _ _ v h
  -- each folded operand value equals its denoted operand value (via IH + canon-stability)
  have hcu0 : c0 = u0 := by
    have hc := ih0 u0 hu0
    rw [hn0] at hc
    simp only [denote] at hc
    have hcanon : (match Value.asF64? c0 with | some f => Value.f64 f | none => c0) = c0 :=
      normalize_const_canon a0 c0 hn0
    rw [hcanon] at hc
    exact Outcome.value.inj hc
  have hcu1 : c1 = u1 := by
    have hc := ih1 u1 hu1
    rw [hn1] at hc
    simp only [denote] at hc
    have hcanon : (match Value.asF64? c1 with | some f => Value.f64 f | none => c1) = c1 :=
      normalize_const_canon a1 c1 hn1
    rw [hcanon] at hc
    exact Outcome.value.inj hc
  -- the fold outcome equals the denoted binary outcome
  have hfoldv : denoteBinary op w (.value u0) (.value u1) = .value vf := by
    rw [← hcu0, ← hcu1]; exact denoteBinary_fold op w c0 c1 vf hfold
  rw [hu0, hu1] at h
  rw [h] at hfoldv
  exact (Outcome.value.inj hfoldv)

theorem denoteApp_normalize_fold_unary (ρ : Nat → Value) (w : IntTy) (op : String)
    (a0 : SymExpr) (v c0 vf : Value)
    (hn0 : normalize a0 = .const c0)
    (hfold : foldConst? op #[.const c0] = some vf)
    (ih0 : ∀ u, denote ρ w a0 = .value u → denote ρ w (normalize a0) = .value u)
    (h : denoteUnary op (denote ρ w a0) = .value v) : v = vf := by
  obtain ⟨u0, hu0⟩ := denoteUnary_value_inv op _ v h
  have hcu0 : c0 = u0 := by
    have hc := ih0 u0 hu0
    rw [hn0] at hc
    simp only [denote] at hc
    have hcanon : (match Value.asF64? c0 with | some f => Value.f64 f | none => c0) = c0 :=
      normalize_const_canon a0 c0 hn0
    rw [hcanon] at hc
    exact Outcome.value.inj hc
  have hfoldv : denoteUnary op (.value u0) = .value vf := by
    rw [← hcu0]; exact denoteUnary_fold op c0 vf hfold
  rw [hu0] at h
  rw [h] at hfoldv
  exact (Outcome.value.inj hfoldv)


/-! ### Capstone compound-congruence cases (trivial: `denote` is `unsupported` on compounds).
`normalize` is a congruence on the value-shaped compounds (rebuilds the same constructor with children
normalized — the structural equations above), and `denote` currently maps every compound to the SAME
`.unsupported` (they are a later `denote` increment), so both sides are that identical `.unsupported`.
These discharge the compound cases of the eventual `denote (normalize e) = denote e` induction; the
`.app`/`.ite` inductive cases remain (⚠ the `.app` algebraic identities like `x+0 → x` make the full
theorem hold only for WELL-TYPED `e` — `denote (+ bool 0)` is `.unsupported` while `normalize (+ bool 0)`
= `bool`; a typing predicate/hypothesis is the design point for that increment, not an oracle bug since
ill-typed programs never reach the differential). -/
-- NOTE: `denote_normalize_tuple` was removed — `denote` now MODELS `.tuple` (evaluates each element via
-- `outcomeToValue`, matching `evalNode`), so the tuple case is no longer a trivial `unsupported = unsupported`
-- congruence: `denote (normalize (.tuple es)) = denote (.tuple es)` now needs the per-element IH
-- (`denote (normalize eᵢ) = denote eᵢ`), i.e. it is subsumed into the eventual capstone induction's tuple
-- case (an `Array.map` congruence over the element IHs). `ctor`/`proj`/`case` remain `unsupported` (below).
theorem denote_normalize_ctor (ρ : Nat → Value) (w : IntTy) (tag : ByteArray) (args : Array SymExpr) :
    denote ρ w (normalize (.ctor tag args)) = denote ρ w (.ctor tag args) := by simp only [normalize, denote]
theorem denote_normalize_proj (ρ : Nat → Value) (w : IntTy) (b : SymExpr) (s : ByteArray) :
    denote ρ w (normalize (.proj b s)) = denote ρ w (.proj b s) := by simp only [normalize, denote]
theorem denote_normalize_case (ρ : Nat → Value) (w : IntTy) (s : SymExpr) (arms : Array (ByteArray × SymExpr)) :
    denote ρ w (normalize (.case s arms)) = denote ρ w (.case s arms) := by simp only [normalize, denote]

/-! ### `denote` equations for the modeled COMPOUND arms (tuple #6569, record #6579).
Pin the value-producing shape of the compound arms `denote` now models: each denotes to a `.value` of the
compound whose children are the elements' `denote` folded through `outcomeToValue` (poison for a trapping
child — "trap when observed", matching `evalNode`). These are the building blocks the eventual capstone
tuple/record congruence cases use (they reduce `denote (.tuple/.record …)` to the element denotations),
and they pin the arm shape against a refactor. -/
/-- `denote`-app REDUCTION for a concrete UNARY/BINARY application: `denote (.app op #[…])` reduces
through `denoteApp`'s arity dispatch (`.attach.map` on the literal + the size-1/2 `if`) to
`denoteUnary`/`denoteBinary` over the operand denotations. This is the GLUE that lets the capstone `.app`
fold-soundness core (`denoteApp_normalize_fold_{binary,unary}`, stated at the `denoteBinary`/`denoteUnary`
level) apply to an actual `.app` node once its args are destructured by arity. -/
theorem denote_app1 (ρ : Nat → Value) (w : IntTy) (op : String) (a0 : SymExpr) :
    denote ρ w (.app op #[a0]) = denoteUnary op (denote ρ w a0) := by
  simp [denote, denoteApp]

theorem denote_app2 (ρ : Nat → Value) (w : IntTy) (op : String) (a0 a1 : SymExpr) :
    denote ρ w (.app op #[a0, a1]) = denoteBinary op w (denote ρ w a0) (denote ρ w a1) := by
  simp [denote, denoteApp]

/-- CAPSTONE `.app` FOLD CASE (concrete binary, scalar/const operands): the value-form soundness
`denote (.app op #[a0,a1]) = .value v → denote (normalize (.app op #[a0,a1])) = .value v` when the
operands normalize to constants and `foldConst?` fires. Ties the glue (`denote_app2`), the fold-soundness
core (`denoteApp_normalize_fold_binary`), `normalize_app_fold` (normalize collapses to `.const vf`), and
`foldConst?_canon_stable` (`denote (.const vf) = .value vf`). The concrete-arity + scalar-operand slice of
the capstone `.app` case; the arbitrary-arity destructure + tuple/record-operand bridge remain. -/
theorem denote_normalize_app_fold2 (ρ : Nat → Value) (w : IntTy) (op : String)
    (a0 a1 : SymExpr) (v c0 c1 vf : Value)
    (hn0 : normalize a0 = .const c0) (hn1 : normalize a1 = .const c1)
    (hfold : foldConst? op #[.const c0, .const c1] = some vf)
    (ih0 : ∀ u, denote ρ w a0 = .value u → denote ρ w (normalize a0) = .value u)
    (ih1 : ∀ u, denote ρ w a1 = .value u → denote ρ w (normalize a1) = .value u)
    (h : denote ρ w (.app op #[a0, a1]) = .value v) :
    denote ρ w (normalize (.app op #[a0, a1])) = .value v := by
  have hargs : (#[a0, a1] : Array SymExpr).attach.map (fun x => normalize x.val) = #[.const c0, .const c1] := by
    simp [hn0, hn1]
  have hnf : normalize (.app op #[a0, a1]) = .const vf := by
    rw [normalize_app_fold op #[a0, a1] vf (by rw [hargs]; exact hfold)]
  rw [hnf]
  have hv : v = vf := by
    rw [denote_app2] at h
    exact denoteApp_normalize_fold_binary ρ w op a0 a1 v c0 c1 vf hn0 hn1 hfold ih0 ih1 h
  simp only [denote]
  rw [foldConst?_canon_stable op #[.const c0, .const c1] vf hfold, hv]

/-- CAPSTONE `.app` FOLD CASE (concrete unary, `not` of a const operand). Arity-1 companion of
`denote_normalize_app_fold2`. -/
theorem denote_normalize_app_fold1 (ρ : Nat → Value) (w : IntTy) (op : String)
    (a0 : SymExpr) (v c0 vf : Value)
    (hn0 : normalize a0 = .const c0)
    (hfold : foldConst? op #[.const c0] = some vf)
    (ih0 : ∀ u, denote ρ w a0 = .value u → denote ρ w (normalize a0) = .value u)
    (h : denote ρ w (.app op #[a0]) = .value v) :
    denote ρ w (normalize (.app op #[a0])) = .value v := by
  have hargs : (#[a0] : Array SymExpr).attach.map (fun x => normalize x.val) = #[.const c0] := by
    simp [hn0]
  have hnf : normalize (.app op #[a0]) = .const vf := by
    rw [normalize_app_fold op #[a0] vf (by rw [hargs]; exact hfold)]
  rw [hnf]
  have hv : v = vf := by
    rw [denote_app1] at h
    exact denoteApp_normalize_fold_unary ρ w op a0 v c0 vf hn0 hfold ih0 h
  simp only [denote]
  rw [foldConst?_canon_stable op #[.const c0] vf hfold, hv]

/-- ARITY REDUCTION for the capstone `.app` case: a `.value`-producing application has arity 1 or 2 (its
args are a concrete singleton / pair), because `denote (.app …)` goes through `denoteApp`, which is
`.unsupported` at any other arity — so a `.value` result (`denoteApp_value_inv`) pins the size, and the
arg array destructures accordingly. This lets the concrete-arity fold cases (`denote_normalize_app_fold1`/
`_fold2`) and the eventual identity case apply to an arbitrary-arity `.app` node in the `denote.induct`. -/
theorem denote_app_value_arity (ρ : Nat → Value) (w : IntTy) (op : String) (args : Array SymExpr) (v : Value)
    (h : denote ρ w (.app op args) = .value v) :
    (∃ a0, args = #[a0]) ∨ (∃ a0 a1, args = #[a0, a1]) := by
  simp only [denote] at h
  rcases denoteApp_value_inv op w _ v h with ⟨hs, _⟩ | ⟨hs, _⟩
  · left
    rw [Array.size_map, Array.size_attach] at hs
    exact Array.size_eq_one_iff.mp hs
  · right
    rw [Array.size_map, Array.size_attach] at hs
    match args, hs with
    | ⟨[a0, a1]⟩, _ => exact ⟨a0, a1, rfl⟩

theorem denote_tuple (ρ : Nat → Value) (w : IntTy) (es : Array SymExpr) :
    denote ρ w (.tuple es) = .value (.tuple (es.attach.map (fun x => outcomeToValue (denote ρ w x.val)))) := by
  simp only [denote]
theorem denote_record (ρ : Nat → Value) (w : IntTy) (fs : Array (ByteArray × SymExpr)) :
    denote ρ w (.record fs)
      = .value (.record (fs.attach.map (fun x => (x.val.1, outcomeToValue (denote ρ w x.val.2))))) := by
  simp only [denote]

/-! ### `symToValue?`↔`denote` bridge — a CONSTANT expression (`.const`/`.tuple`/`.record` of constants)
whose const leaves are `asF64?`-canonical (`AllConstsCanon`) `denote`s to EXACTLY the value `symToValue?`
extracts. This closes the general (tuple/record-operand) `.app` fold case: a folded operand's `symToValue?`
value equals its `denote` value, so the fold outcome matches. The compound cases need a generic
`mapM (Option)` ↔ `map` agreement lemma (`array_mapM_map_agree`), proven by `List.mapM_cons` induction. -/
private theorem list_mapM_map_agree {α β} (f : α → Option β) (g : α → β) :
    ∀ (l : List α) (out : List β), l.mapM f = some out →
      (∀ x ∈ l, ∀ y, f x = some y → g x = y) → l.map g = out := by
  intro l
  induction l with
  | nil => intro out h _; simp_all [List.mapM_nil]
  | cons a as ih =>
    intro out h hfg
    rw [List.mapM_cons] at h
    cases hfa : f a with
    | none => simp [hfa] at h
    | some b =>
      cases hbs : as.mapM f with
      | none => simp [hfa, hbs] at h
      | some bs =>
        simp only [hfa, hbs, Option.bind_some, Option.pure_def] at h
        injection h with h'; rw [← h']
        have hga : g a = b := hfg a (by simp) b hfa
        have hgas : as.map g = bs := ih bs hbs (fun x hx => hfg x (by simp [hx]))
        simp [List.map_cons, hga, hgas]

private theorem array_mapM_map_agree {α β} (f : α → Option β) (g : α → β) (arr : Array α) (out : Array β)
    (h : arr.mapM f = some out) (hfg : ∀ x ∈ arr, ∀ y, f x = some y → g x = y) :
    arr.map g = out := by
  have hconv : arr.toList.mapM f = some out.toList := by
    have h2 : (List.toArray <$> arr.toList.mapM f) = some out := by
      rw [← List.mapM_toArray, Array.toArray_toList]; exact h
    cases hl : arr.toList.mapM f with
    | none => rw [hl] at h2; simp at h2
    | some ol => rw [hl] at h2; injection h2 with h2'; rw [← h2']
  apply Array.toList_inj.mp
  rw [Array.toList_map]
  exact list_mapM_map_agree f g arr.toList out.toList hconv (fun x hx => hfg x (by simpa using hx))

/-- The bridge: an `AllConstsCanon` expression `denote`s to the value `symToValue?` extracts. -/
theorem denote_symToValue (ρ : Nat → Value) (w : IntTy) (e : SymExpr) (c : Value)
    (hcanon : AllConstsCanon e) (h : symToValue? e = some c) : denote ρ w e = .value c := by
  induction e using symToValue?.induct generalizing c with
  | case1 v =>
    simp only [symToValue?] at h; injection h with h'; subst h'
    simp only [denote]
    simp only [AllConstsCanon] at hcanon
    rw [hcanon]
  | case2 es ih =>
    simp only [symToValue?] at h
    cases hm : es.attach.mapM (fun x => symToValue? x.val) with
    | none => rw [hm] at h; simp at h
    | some cs =>
      rw [hm] at h; simp only [Option.map_some, Option.some.injEq] at h; subst h
      rw [denote_tuple]
      simp only [Outcome.value.injEq, Value.tuple.injEq]
      apply array_mapM_map_agree (fun x => symToValue? x.val)
        (fun x => outcomeToValue (denote ρ w x.val)) es.attach cs hm
      intro x _ y hy
      simp only [AllConstsCanon] at hcanon
      rw [ih x y (hcanon x.val x.property) hy]; rfl
  | case3 fs ih =>
    simp only [symToValue?] at h
    cases hm : fs.attach.mapM (fun x => (symToValue? x.val.2).map (fun v => (x.val.1, v))) with
    | none => rw [hm] at h; simp at h
    | some cs =>
      rw [hm] at h; simp only [Option.map_some, Option.some.injEq] at h; subst h
      rw [denote_record]
      simp only [Outcome.value.injEq, Value.record.injEq]
      apply array_mapM_map_agree (fun x => (symToValue? x.val.2).map (fun v => (x.val.1, v)))
        (fun x => (x.val.1, outcomeToValue (denote ρ w x.val.2))) fs.attach cs hm
      intro x _ y hy
      simp only [AllConstsCanon] at hcanon
      cases hs : symToValue? x.val.2 with
      | none => rw [hs] at hy; simp at hy
      | some d =>
        rw [hs] at hy; simp only [Option.map_some, Option.some.injEq] at hy; subst hy
        rw [ih x d (hcanon x.val.1 x.val.2 (by simpa using x.property)) hs]; rfl
  | case4 x _ _ _ =>
    exfalso
    cases x <;> simp_all [symToValue?]

/-- CAPSTONE `.app` FOLD CASE — GENERAL (arbitrary arity, scalar OR tuple/record operands). When
`normalize` folds an application to a constant `vf`, the original application `denote`s to that same `vf`.
Uniform over all operand shapes via the `symToValue?`↔`denote` bridge (`denote_symToValue`): each folded
operand's `symToValue?` value equals its `denote` value (no `.const`-vs-`.tuple` split). The value hypothesis
pins arity 1/2 (`denote_app_value_arity`), the operands are `.value` (`denote{Unary,Binary}_value_inv`) and
`symToValue?`-extractable (`foldConst?_*_operand*_some`), and `foldConst?_symToValue_congr` swaps each
normalized operand for `.const (its value)` to reach `denote{Unary,Binary}_fold`. This is the FULL `.app`
fold branch of the `denote (normalize e) = denote e` capstone. -/
theorem denote_normalize_app_fold (ρ : Nat → Value) (w : IntTy) (op : String) (args : Array SymExpr)
    (vf v : Value)
    (hfold : foldConst? op (args.attach.map (fun x => normalize x.val)) = some vf)
    (ih : ∀ x ∈ args, ∀ u, denote ρ w x = .value u → denote ρ w (normalize x) = .value u)
    (h : denote ρ w (.app op args) = .value v) :
    denote ρ w (normalize (.app op args)) = .value v := by
  rw [normalize_app_fold op args vf hfold]
  simp only [denote]
  rw [foldConst?_canon_stable op _ vf hfold]
  suffices hv : v = vf by rw [hv]
  rcases denote_app_value_arity ρ w op args v h with ⟨a0, rfl⟩ | ⟨a0, a1, rfl⟩
  · -- UNARY: args = #[a0]
    rw [show (#[a0] : Array SymExpr).attach.map (fun x => normalize x.val) = #[normalize a0] by simp] at hfold
    obtain ⟨c0, hc0⟩ := foldConst?_unary_operand_some op (normalize a0) vf hfold
    have hd0 : denote ρ w (normalize a0) = .value c0 :=
      denote_symToValue ρ w (normalize a0) c0 (normalize_allConstsCanon a0) hc0
    rw [denote_app1] at h
    obtain ⟨u0, hu0⟩ := denoteUnary_value_inv op _ v h
    have hi0 : denote ρ w (normalize a0) = .value u0 := ih a0 (by simp) u0 hu0
    have hcu0 : c0 = u0 := by rw [hd0] at hi0; exact Outcome.value.inj hi0
    have hcongr : foldConst? op #[normalize a0] = foldConst? op #[.const c0] :=
      foldConst?_symToValue_congr op _ _ (by simp [hc0, symToValue?_const])
    rw [hcongr] at hfold
    have hfv : denoteUnary op (.value c0) = .value vf := denoteUnary_fold op c0 vf hfold
    rw [hu0, ← hcu0, hfv] at h
    exact (Outcome.value.inj h).symm
  · -- BINARY: args = #[a0, a1]
    rw [show (#[a0, a1] : Array SymExpr).attach.map (fun x => normalize x.val)
        = #[normalize a0, normalize a1] by simp] at hfold
    obtain ⟨⟨c0, hc0⟩, ⟨c1, hc1⟩⟩ := foldConst?_binary_operands_some op (normalize a0) (normalize a1) vf hfold
    have hd0 : denote ρ w (normalize a0) = .value c0 :=
      denote_symToValue ρ w (normalize a0) c0 (normalize_allConstsCanon a0) hc0
    have hd1 : denote ρ w (normalize a1) = .value c1 :=
      denote_symToValue ρ w (normalize a1) c1 (normalize_allConstsCanon a1) hc1
    rw [denote_app2] at h
    obtain ⟨⟨u0, hu0⟩, ⟨u1, hu1⟩⟩ := denoteBinary_value_inv op w _ _ v h
    have hcu0 : c0 = u0 := by
      have := ih a0 (by simp) u0 hu0; rw [hd0] at this; exact Outcome.value.inj this
    have hcu1 : c1 = u1 := by
      have := ih a1 (by simp) u1 hu1; rw [hd1] at this; exact Outcome.value.inj this
    have hcongr : foldConst? op #[normalize a0, normalize a1] = foldConst? op #[.const c0, .const c1] :=
      foldConst?_symToValue_congr op _ _ (by simp [hc0, hc1, symToValue?_const])
    rw [hcongr] at hfold
    have hfv : denoteBinary op w (.value c0) (.value c1) = .value vf := denoteBinary_fold op w c0 c1 vf hfold
    rw [hu0, hu1, ← hcu0, ← hcu1, hfv] at h
    exact (Outcome.value.inj h).symm

/-- CAPSTONE `.app` IDENT branch — PLAIN fall-through (`foldConst? = none` AND no algebraic identity fires,
so `normalize` rebuilds `.app op (map normalize args)`). The dominant `.app` case (most applications match
no fold/identity pattern). denote is preserved by per-argument congruence: the value hypothesis pins arity,
each operand denotes to a value, and the argument IHs carry `denote aᵢ = denote (normalize aᵢ)`. Takes the
plain shape as a hypothesis (`hplain`), which the eventual `denote.induct` assembly discharges in the
fall-through sub-case. -/
theorem denote_normalize_app_ident_plain (ρ : Nat → Value) (w : IntTy) (op : String)
    (args : Array SymExpr) (v : Value)
    (hplain : normalize (.app op args) = .app op (args.attach.map (fun x => normalize x.val)))
    (ih : ∀ x ∈ args, ∀ u, denote ρ w x = .value u → denote ρ w (normalize x) = .value u)
    (h : denote ρ w (.app op args) = .value v) :
    denote ρ w (normalize (.app op args)) = .value v := by
  rw [hplain]
  rcases denote_app_value_arity ρ w op args v h with ⟨a0, rfl⟩ | ⟨a0, a1, rfl⟩
  · rw [show (#[a0] : Array SymExpr).attach.map (fun x => normalize x.val) = #[normalize a0] by simp]
    rw [denote_app1] at h ⊢
    obtain ⟨u0, hu0⟩ := denoteUnary_value_inv op _ v h
    rw [hu0] at h
    rw [ih a0 (by simp) u0 hu0]
    exact h
  · rw [show (#[a0, a1] : Array SymExpr).attach.map (fun x => normalize x.val)
        = #[normalize a0, normalize a1] by simp]
    rw [denote_app2] at h ⊢
    obtain ⟨⟨u0, hu0⟩, ⟨u1, hu1⟩⟩ := denoteBinary_value_inv op w _ _ v h
    rw [hu0, hu1] at h
    rw [ih a0 (by simp) u0 hu0, ih a1 (by simp) u1 hu1]
    exact h

/-- `denote (.const (.int n)) = .value (.int n)` (an int const is `asF64?`-canonical, so denote's float
canonicalization is the identity on it). -/
theorem denote_const_int (ρ : Nat → Value) (w : IntTy) (n : Int) :
    denote ρ w (.const (.int n)) = .value (.int n) := by simp [denote, Value.asF64?]

/-- `foldConst?` NEVER folds `+ x (int 0)` — its arith arm folds only via `asF64?`, and `asF64? (.int 0)`
is `none`, so the fold declines for ANY first operand. This is exactly the `foldConst? = none` the `x+0`
identity case has in hand (the identity fires only after `foldConst?` declined), for the specific `.const`
operands `denoteBinary_arith_int_r`/`denoteBinary_add_zero_value` need. -/
theorem foldConst_add_int0_none (va : Value) : foldConst? "+" #[.const va, .const (.int 0)] = none := by
  simp [foldConst?, symToValue?, Value.asF64?]

/-- CAPSTONE `.app`-IDENTITY case — `x + 0 → x` (operand-preserving, the FIRST assembled algebraic-identity
denote-soundness case, validating the #7216 opaque-BEq unblock composes END-TO-END). `foldConst?` declined
(`hnone`) so `normalize` took the identity branch, and the 2nd operand normalized to `.const (.int 0)`
(`hn1`), so `normalizeAppIdentities` returns `normalize a0` (`normalizeAppIdentities_add_zero_r`, now
reducible via `isConstInt`). Soundness: from `h`, both operands denote to values (`denoteBinary_value_inv`);
`ih1` + `hn1` pin the 2nd to `.int 0` (`denote_const_int`); the arith result forces the 1st to `.int x`
(`denoteBinary_arith_int_r`); `denoteBinary_add_zero_value` gives `v = .int x`; `ih0` carries
`denote (normalize a0) = .value (.int x) = .value v`. The other operand-preserving identities
(`x-0`, `x*1`, `1*x`, `x/1`) assemble identically with their `normalizeAppIdentities_*`/`denoteBinary_*_value`
pair; operand-DROPPING (`x*0`/`x%1`) need the `!mayTrap` route. -/
theorem denote_normalize_app_ident_add_zero_r (ρ : Nat → Value) (w : IntTy) (a0 a1 : SymExpr) (v : Value)
    (hn1 : normalize a1 = .const (.int 0))
    (hnone : foldConst? "+" (#[a0, a1].attach.map (fun x => normalize x.val)) = none)
    (ih1 : ∀ u, denote ρ w a1 = .value u → denote ρ w (normalize a1) = .value u)
    (ih0 : ∀ u, denote ρ w a0 = .value u → denote ρ w (normalize a0) = .value u)
    (h : denote ρ w (.app "+" #[a0, a1]) = .value v) :
    denote ρ w (normalize (.app "+" #[a0, a1])) = .value v := by
  rw [normalize_app_ident "+" #[a0, a1] hnone]
  rw [show (#[a0, a1] : Array SymExpr).attach.map (fun x => normalize x.val) = #[normalize a0, normalize a1] by simp]
  rw [hn1, normalizeAppIdentities_add_zero_r (normalize a0)]
  rw [denote_app2] at h
  obtain ⟨⟨u0, hu0⟩, ⟨u1, hu1⟩⟩ := denoteBinary_value_inv "+" w _ _ v h
  rw [hu0, hu1] at h
  have hu1' : u1 = .int 0 := by
    have := ih1 u1 hu1; rw [hn1, denote_const_int] at this
    exact (Outcome.value.inj this).symm
  subst hu1'
  obtain ⟨x, rfl⟩ := denoteBinary_arith_int_r "+" w u0 0 v (foldConst_add_int0_none u0) (by decide) h
  have hvx : v = .int x := denoteBinary_add_zero_value w x v (foldConst_add_int0_none _) h
  rw [ih0 (.int x) hu0, hvx]

/-- `foldConst?` never folds an arith/`%` op with a RIGHT int operand (`asF64?` of an int is `none` → the
arith arm declines; `%` has no `foldConst?` arm at all). The `foldConst? = none` the operand-preserving
RIGHT-identity cases (`x-0`, `x*1`, `x/1`) need for their `.const` operands. -/
theorem foldConst_arith_int_none (op : String) (va : Value) (n : Int)
    (hop : op = "+" ∨ op = "-" ∨ op = "*" ∨ op = "/" ∨ op = "%") :
    foldConst? op #[.const va, .const (.int n)] = none := by
  rcases hop with h|h|h|h|h <;> subst h <;> simp [foldConst?, symToValue?, Value.asF64?]
/-- LEFT companion of `foldConst_arith_int_none` (int operand on the LEFT; for `0+x`, `1*x`). -/
theorem foldConst_arith_int_none_l (op : String) (n : Int) (vb : Value)
    (hop : op = "+" ∨ op = "-" ∨ op = "*" ∨ op = "/" ∨ op = "%") :
    foldConst? op #[.const (.int n), .const vb] = none := by
  rcases hop with h|h|h|h|h <;> subst h <;> simp [foldConst?, symToValue?, Value.asF64?]

/-- CAPSTONE `.app`-IDENTITY: `x - 0 → x` (same assembly as `x+0`, op `-`). -/
theorem denote_normalize_app_ident_sub_zero_r (ρ : Nat → Value) (w : IntTy) (a0 a1 : SymExpr) (v : Value)
    (hn1 : normalize a1 = .const (.int 0))
    (hnone : foldConst? "-" (#[a0, a1].attach.map (fun x => normalize x.val)) = none)
    (ih1 : ∀ u, denote ρ w a1 = .value u → denote ρ w (normalize a1) = .value u)
    (ih0 : ∀ u, denote ρ w a0 = .value u → denote ρ w (normalize a0) = .value u)
    (h : denote ρ w (.app "-" #[a0, a1]) = .value v) :
    denote ρ w (normalize (.app "-" #[a0, a1])) = .value v := by
  rw [normalize_app_ident "-" #[a0, a1] hnone,
      show (#[a0, a1] : Array SymExpr).attach.map (fun x => normalize x.val) = #[normalize a0, normalize a1] by simp,
      hn1, normalizeAppIdentities_sub_zero_r (normalize a0)]
  rw [denote_app2] at h
  obtain ⟨⟨u0, hu0⟩, ⟨u1, hu1⟩⟩ := denoteBinary_value_inv "-" w _ _ v h
  rw [hu0, hu1] at h
  have hu1' : u1 = .int 0 := by
    have := ih1 u1 hu1; rw [hn1, denote_const_int] at this; exact (Outcome.value.inj this).symm
  subst hu1'
  obtain ⟨x, rfl⟩ := denoteBinary_arith_int_r "-" w u0 0 v (foldConst_arith_int_none "-" u0 0 (by decide)) (by decide) h
  rw [ih0 (.int x) hu0, denoteBinary_sub_zero_value w x v (foldConst_arith_int_none "-" (.int x) 0 (by decide)) h]

/-- CAPSTONE `.app`-IDENTITY: `x * 1 → x`. -/
theorem denote_normalize_app_ident_mul_one_r (ρ : Nat → Value) (w : IntTy) (a0 a1 : SymExpr) (v : Value)
    (hn1 : normalize a1 = .const (.int 1))
    (hnone : foldConst? "*" (#[a0, a1].attach.map (fun x => normalize x.val)) = none)
    (ih1 : ∀ u, denote ρ w a1 = .value u → denote ρ w (normalize a1) = .value u)
    (ih0 : ∀ u, denote ρ w a0 = .value u → denote ρ w (normalize a0) = .value u)
    (h : denote ρ w (.app "*" #[a0, a1]) = .value v) :
    denote ρ w (normalize (.app "*" #[a0, a1])) = .value v := by
  rw [normalize_app_ident "*" #[a0, a1] hnone,
      show (#[a0, a1] : Array SymExpr).attach.map (fun x => normalize x.val) = #[normalize a0, normalize a1] by simp,
      hn1, normalizeAppIdentities_mul_one_r (normalize a0)]
  rw [denote_app2] at h
  obtain ⟨⟨u0, hu0⟩, ⟨u1, hu1⟩⟩ := denoteBinary_value_inv "*" w _ _ v h
  rw [hu0, hu1] at h
  have hu1' : u1 = .int 1 := by
    have := ih1 u1 hu1; rw [hn1, denote_const_int] at this; exact (Outcome.value.inj this).symm
  subst hu1'
  obtain ⟨x, rfl⟩ := denoteBinary_arith_int_r "*" w u0 1 v (foldConst_arith_int_none "*" u0 1 (by decide)) (by decide) h
  rw [ih0 (.int x) hu0, denoteBinary_mul_one_r_value w x v (foldConst_arith_int_none "*" (.int x) 1 (by decide)) h]

/-- CAPSTONE `.app`-IDENTITY: `x / 1 → x`. -/
theorem denote_normalize_app_ident_div_one_r (ρ : Nat → Value) (w : IntTy) (a0 a1 : SymExpr) (v : Value)
    (hn1 : normalize a1 = .const (.int 1))
    (hnone : foldConst? "/" (#[a0, a1].attach.map (fun x => normalize x.val)) = none)
    (ih1 : ∀ u, denote ρ w a1 = .value u → denote ρ w (normalize a1) = .value u)
    (ih0 : ∀ u, denote ρ w a0 = .value u → denote ρ w (normalize a0) = .value u)
    (h : denote ρ w (.app "/" #[a0, a1]) = .value v) :
    denote ρ w (normalize (.app "/" #[a0, a1])) = .value v := by
  rw [normalize_app_ident "/" #[a0, a1] hnone,
      show (#[a0, a1] : Array SymExpr).attach.map (fun x => normalize x.val) = #[normalize a0, normalize a1] by simp,
      hn1, normalizeAppIdentities_div_one_r (normalize a0)]
  rw [denote_app2] at h
  obtain ⟨⟨u0, hu0⟩, ⟨u1, hu1⟩⟩ := denoteBinary_value_inv "/" w _ _ v h
  rw [hu0, hu1] at h
  have hu1' : u1 = .int 1 := by
    have := ih1 u1 hu1; rw [hn1, denote_const_int] at this; exact (Outcome.value.inj this).symm
  subst hu1'
  obtain ⟨x, rfl⟩ := denoteBinary_arith_int_r "/" w u0 1 v (foldConst_arith_int_none "/" u0 1 (by decide)) (by decide) h
  rw [ih0 (.int x) hu0, denoteBinary_div_one_value w x v (foldConst_arith_int_none "/" (.int x) 1 (by decide)) h]

/-- CAPSTONE `.app`-IDENTITY (LEFT): `0 + x → x`. The 2nd operand survives (`hb`: it is not itself the
literal 0, else the RIGHT identity would fire first). Mirrors the `_r` cases with operand roles swapped
(via `denoteBinary_arith_int_l` + `denoteBinary_add_zero_l_value` + `ih1`). -/
theorem denote_normalize_app_ident_add_zero_l (ρ : Nat → Value) (w : IntTy) (a0 a1 : SymExpr) (v : Value)
    (hn0 : normalize a0 = .const (.int 0)) (hb : isConstInt (normalize a1) 0 = false)
    (hnone : foldConst? "+" (#[a0, a1].attach.map (fun x => normalize x.val)) = none)
    (ih1 : ∀ u, denote ρ w a1 = .value u → denote ρ w (normalize a1) = .value u)
    (ih0 : ∀ u, denote ρ w a0 = .value u → denote ρ w (normalize a0) = .value u)
    (h : denote ρ w (.app "+" #[a0, a1]) = .value v) :
    denote ρ w (normalize (.app "+" #[a0, a1])) = .value v := by
  rw [normalize_app_ident "+" #[a0, a1] hnone,
      show (#[a0, a1] : Array SymExpr).attach.map (fun x => normalize x.val) = #[normalize a0, normalize a1] by simp,
      hn0, normalizeAppIdentities_add_zero_l (normalize a1) hb]
  rw [denote_app2] at h
  obtain ⟨⟨u0, hu0⟩, ⟨u1, hu1⟩⟩ := denoteBinary_value_inv "+" w _ _ v h
  rw [hu0, hu1] at h
  have hu0' : u0 = .int 0 := by
    have := ih0 u0 hu0; rw [hn0, denote_const_int] at this; exact (Outcome.value.inj this).symm
  subst hu0'
  obtain ⟨y, rfl⟩ := denoteBinary_arith_int_l "+" w 0 u1 v (foldConst_arith_int_none_l "+" 0 u1 (by decide)) (by decide) h
  rw [ih1 (.int y) hu1, denoteBinary_add_zero_l_value w y v (foldConst_arith_int_none_l "+" 0 (.int y) (by decide)) h]

/-- CAPSTONE `.app`-IDENTITY (LEFT): `1 * x → x`. -/
theorem denote_normalize_app_ident_mul_one_l (ρ : Nat → Value) (w : IntTy) (a0 a1 : SymExpr) (v : Value)
    (hn0 : normalize a0 = .const (.int 1)) (hb : isConstInt (normalize a1) 1 = false)
    (hnone : foldConst? "*" (#[a0, a1].attach.map (fun x => normalize x.val)) = none)
    (ih1 : ∀ u, denote ρ w a1 = .value u → denote ρ w (normalize a1) = .value u)
    (ih0 : ∀ u, denote ρ w a0 = .value u → denote ρ w (normalize a0) = .value u)
    (h : denote ρ w (.app "*" #[a0, a1]) = .value v) :
    denote ρ w (normalize (.app "*" #[a0, a1])) = .value v := by
  rw [normalize_app_ident "*" #[a0, a1] hnone,
      show (#[a0, a1] : Array SymExpr).attach.map (fun x => normalize x.val) = #[normalize a0, normalize a1] by simp,
      hn0, normalizeAppIdentities_mul_one_l (normalize a1) hb]
  rw [denote_app2] at h
  obtain ⟨⟨u0, hu0⟩, ⟨u1, hu1⟩⟩ := denoteBinary_value_inv "*" w _ _ v h
  rw [hu0, hu1] at h
  have hu0' : u0 = .int 1 := by
    have := ih0 u0 hu0; rw [hn0, denote_const_int] at this; exact (Outcome.value.inj this).symm
  subst hu0'
  obtain ⟨y, rfl⟩ := denoteBinary_arith_int_l "*" w 1 u1 v (foldConst_arith_int_none_l "*" 1 u1 (by decide)) (by decide) h
  rw [ih1 (.int y) hu1, denoteBinary_mul_one_l_value w y v (foldConst_arith_int_none_l "*" 1 (.int y) (by decide)) h]

/-! ### Operand-DROPPING `.app`-identity denote-soundness (`x*0→0`, `0*x→0`, `x%1→0`). The identity
returns `.const (.int 0)` (dropping the surviving operand), guarded by `!mayTrap` on the dropped operand.
Assembly like the preserving cases, but the RESULT is `.const (.int 0)` (no operand IH needed for the
dropped side — the value hypothesis `h` still forces the operands to `.int`, and the `denoteBinary_*_value`
characterization gives `v = .int 0`, so `denote (.const (.int 0)) = .value (.int 0) = .value v`). The
`!mayTrap` guard is carried as `hmt` (from the branch condition in the eventual assembly); it is what lets
`normalizeAppIdentities` fire, and is faithful to normalize's soundness rule (dropping a trapping operand
would be unsound, but `h` already witnesses no trap occurred). -/
theorem normalizeAppIdentities_mul_zero_r (a : SymExpr) (hmt : mayTrap a = false) :
    normalizeAppIdentities "*" #[a, .const (.int 0)] = .const (.int 0) := by
  simp [normalizeAppIdentities, isConstInt, hmt]
theorem normalizeAppIdentities_mul_zero_l (b : SymExpr) (hmt : mayTrap b = false) :
    normalizeAppIdentities "*" #[.const (.int 0), b] = .const (.int 0) := by
  simp [normalizeAppIdentities, isConstInt, hmt]
theorem normalizeAppIdentities_mod_one_r (a : SymExpr) (hmt : mayTrap a = false) :
    normalizeAppIdentities "%" #[a, .const (.int 1)] = .const (.int 0) := by
  simp [normalizeAppIdentities, isConstInt, hmt]

/-- CAPSTONE `.app`-IDENTITY (DROPPING): `x * 0 → 0`. -/
theorem denote_normalize_app_ident_mul_zero_r (ρ : Nat → Value) (w : IntTy) (a0 a1 : SymExpr) (v : Value)
    (hn1 : normalize a1 = .const (.int 0)) (hmt : mayTrap (normalize a0) = false)
    (hnone : foldConst? "*" (#[a0, a1].attach.map (fun x => normalize x.val)) = none)
    (ih1 : ∀ u, denote ρ w a1 = .value u → denote ρ w (normalize a1) = .value u)
    (h : denote ρ w (.app "*" #[a0, a1]) = .value v) :
    denote ρ w (normalize (.app "*" #[a0, a1])) = .value v := by
  rw [normalize_app_ident "*" #[a0, a1] hnone,
      show (#[a0, a1] : Array SymExpr).attach.map (fun x => normalize x.val) = #[normalize a0, normalize a1] by simp,
      hn1, normalizeAppIdentities_mul_zero_r (normalize a0) hmt]
  rw [denote_app2] at h
  obtain ⟨⟨u0, hu0⟩, ⟨u1, hu1⟩⟩ := denoteBinary_value_inv "*" w _ _ v h
  rw [hu0, hu1] at h
  have hu1' : u1 = .int 0 := by
    have := ih1 u1 hu1; rw [hn1, denote_const_int] at this; exact (Outcome.value.inj this).symm
  subst hu1'
  obtain ⟨x, rfl⟩ := denoteBinary_arith_int_r "*" w u0 0 v (foldConst_arith_int_none "*" u0 0 (by decide)) (by decide) h
  have hv0 : v = .int 0 := denoteBinary_mul_zero_value w x v (foldConst_arith_int_none "*" (.int x) 0 (by decide)) h
  rw [denote_const_int, hv0]

/-- CAPSTONE `.app`-IDENTITY (DROPPING, LEFT): `0 * x → 0`. -/
theorem denote_normalize_app_ident_mul_zero_l (ρ : Nat → Value) (w : IntTy) (a0 a1 : SymExpr) (v : Value)
    (hn0 : normalize a0 = .const (.int 0)) (hmt : mayTrap (normalize a1) = false)
    (hnone : foldConst? "*" (#[a0, a1].attach.map (fun x => normalize x.val)) = none)
    (ih0 : ∀ u, denote ρ w a0 = .value u → denote ρ w (normalize a0) = .value u)
    (h : denote ρ w (.app "*" #[a0, a1]) = .value v) :
    denote ρ w (normalize (.app "*" #[a0, a1])) = .value v := by
  rw [normalize_app_ident "*" #[a0, a1] hnone,
      show (#[a0, a1] : Array SymExpr).attach.map (fun x => normalize x.val) = #[normalize a0, normalize a1] by simp,
      hn0, normalizeAppIdentities_mul_zero_l (normalize a1) hmt]
  rw [denote_app2] at h
  obtain ⟨⟨u0, hu0⟩, ⟨u1, hu1⟩⟩ := denoteBinary_value_inv "*" w _ _ v h
  rw [hu0, hu1] at h
  have hu0' : u0 = .int 0 := by
    have := ih0 u0 hu0; rw [hn0, denote_const_int] at this; exact (Outcome.value.inj this).symm
  subst hu0'
  obtain ⟨y, rfl⟩ := denoteBinary_arith_int_l "*" w 0 u1 v (foldConst_arith_int_none_l "*" 0 u1 (by decide)) (by decide) h
  have hv0 : v = .int 0 := denoteBinary_mul_zero_l_value w y v (foldConst_arith_int_none_l "*" 0 (.int y) (by decide)) h
  rw [denote_const_int, hv0]

/-- CAPSTONE `.app`-IDENTITY (DROPPING): `x % 1 → 0`. -/
theorem denote_normalize_app_ident_mod_one_r (ρ : Nat → Value) (w : IntTy) (a0 a1 : SymExpr) (v : Value)
    (hn1 : normalize a1 = .const (.int 1)) (hmt : mayTrap (normalize a0) = false)
    (hnone : foldConst? "%" (#[a0, a1].attach.map (fun x => normalize x.val)) = none)
    (ih1 : ∀ u, denote ρ w a1 = .value u → denote ρ w (normalize a1) = .value u)
    (h : denote ρ w (.app "%" #[a0, a1]) = .value v) :
    denote ρ w (normalize (.app "%" #[a0, a1])) = .value v := by
  rw [normalize_app_ident "%" #[a0, a1] hnone,
      show (#[a0, a1] : Array SymExpr).attach.map (fun x => normalize x.val) = #[normalize a0, normalize a1] by simp,
      hn1, normalizeAppIdentities_mod_one_r (normalize a0) hmt]
  rw [denote_app2] at h
  obtain ⟨⟨u0, hu0⟩, ⟨u1, hu1⟩⟩ := denoteBinary_value_inv "%" w _ _ v h
  rw [hu0, hu1] at h
  have hu1' : u1 = .int 1 := by
    have := ih1 u1 hu1; rw [hn1, denote_const_int] at this; exact (Outcome.value.inj this).symm
  subst hu1'
  obtain ⟨x, rfl⟩ := denoteBinary_arith_int_r "%" w u0 1 v (foldConst_arith_int_none "%" u0 1 (by decide)) (by decide) h
  have hv0 : v = .int 0 := denoteBinary_mod_one_value w x v (foldConst_arith_int_none "%" (.int x) 1 (by decide)) h
  rw [denote_const_int, hv0]

/-- `normalizeAppIdentities` DOUBLE-NEGATION rewrite: `not (not x) → x` (returns the grandchild). Reducible
(the `o == "not"` head check is `String` `==`, not the opaque SymExpr `==`). -/
theorem normalizeAppIdentities_not_not (inner : SymExpr) :
    normalizeAppIdentities "not" #[.app "not" #[inner]] = inner := by simp [normalizeAppIdentities]

/-- Inverse of `denoteUnary "not"`: a value-producing `not` had a BOOL operand, and its result is the
negation. -/
theorem denoteUnary_not_inv (u v : Value) (h : denoteUnary "not" (.value u) = .value v) :
    ∃ b, u = .bool b ∧ v = .bool (!b) := by
  cases u <;> simp_all [denoteUnary, foldConst?, symToValue?] <;>
    first | (rename_i b; exact ⟨b, rfl, by simp_all⟩) | done

/-- CAPSTONE `.app`-IDENTITY: DOUBLE-NEGATION `not (not x) → x` (operand-preserving through the inner `not`).
`normalize a0` is the inner `.app "not" #[inner]`, so `normalizeAppIdentities` returns `inner`. Soundness:
peel the OUTER `not` on the original (`denote_app1` + `denoteUnary_not_inv`) — `denote a0 = .value (.bool b0)`,
`v = !b0`; the IH carries `denote (normalize a0) = .value (.bool b0)`, and peeling the inner `not` on
`.app "not" #[inner]` gives `denote inner = .value (.bool (!b0)) = .value v`. -/
theorem denote_normalize_app_ident_not_not (ρ : Nat → Value) (w : IntTy) (a0 inner : SymExpr) (v : Value)
    (hn0 : normalize a0 = .app "not" #[inner])
    (hnone : foldConst? "not" (#[a0].attach.map (fun x => normalize x.val)) = none)
    (ih0 : ∀ u, denote ρ w a0 = .value u → denote ρ w (normalize a0) = .value u)
    (h : denote ρ w (.app "not" #[a0]) = .value v) :
    denote ρ w (normalize (.app "not" #[a0])) = .value v := by
  rw [normalize_app_ident "not" #[a0] hnone,
      show (#[a0] : Array SymExpr).attach.map (fun x => normalize x.val) = #[normalize a0] by simp,
      hn0, normalizeAppIdentities_not_not inner]
  rw [denote_app1] at h
  obtain ⟨u0, hu0⟩ := denoteUnary_value_inv "not" _ v h
  rw [hu0] at h
  obtain ⟨b0, rfl, rfl⟩ := denoteUnary_not_inv u0 v h
  have hna := ih0 (.bool b0) hu0
  rw [hn0, denote_app1] at hna
  obtain ⟨u1, hu1⟩ := denoteUnary_value_inv "not" _ (.bool b0) hna
  rw [hu1] at hna
  obtain ⟨b1, rfl, hb⟩ := denoteUnary_not_inv u1 (.bool b0) hna
  rw [hu1]; simp_all

/-! ### BOOL `.app`-identity denote-soundness (`foldConst?`'s and/or fold, now reducible via `valIsBool` #7243).
The LEFT-const bool identities fire on the FIRST operand, so their `normalizeAppIdentities` equation lemmas
are UNCONDITIONAL. `denote (.const (.bool b)) = .value (.bool b)` (`denote_const_bool`) and
`valIsBool v b = true → v = .bool b` (`valIsBool_eq`) are the reusable bridges. -/
theorem denote_const_bool (ρ : Nat → Value) (w : IntTy) (b : Bool) :
    denote ρ w (.const (.bool b)) = .value (.bool b) := by simp [denote, Value.asF64?]
theorem valIsBool_eq (v : Value) (b : Bool) (h : valIsBool v b = true) : v = .bool b := by
  unfold valIsBool at h; split at h
  · rename_i c; simp only [beq_iff_eq] at h; subst h; rfl
  · exact absurd h (by simp)

/-- SOUNDNESS of the reducible `valEqB` (Symbolic.lean): `valEqB a b = true → a = b`. UNCONDITIONAL — the
`false`-on-float design means a `true` result never involved a float, so structural equality is propositional
equality. This is the leaf primitive for the eventual reducible SymExpr equality that discharges the
capstone's `.app`-IDENTITY IDEMPOTENCE case (`x or x → x`) — the derived `Value`/`SymExpr` BEq is `opaque`,
so its `a == b` guard cannot be reduced in a proof; `valEqB` can. Proven by `valEqB.induct`; ByteArray goals
close via `ByteArray.ext_iff` + `eq_of_beq` on `.data` (`Array UInt8` is `LawfulBEq`; `ByteArray` is not). -/
theorem valEqB_sound (a b : Value) (h : valEqB a b = true) : a = b := by
  induction a, b using valEqB.induct <;>
    simp_all [valEqB, Bool.and_eq_true, ByteArray.ext_iff] <;>
    (first
      | exact eq_of_beq (by assumption)
      | (rename_i hh; exact ⟨eq_of_beq hh.1, hh.2⟩))

/-- Element-wise `symExprEqB` (+ equal sizes) ⇒ the two arrays are propositionally equal. The shared
`Array.ext` step for the app/tuple/ctor cases of `symExprEqB_sound`; `Array.all_eq_true`'s index form gives
`symExprEqB a1[i] a2[i] = true` per index, then the per-element IH closes each. -/
theorem arrEqB_sound (a1 a2 : Array SymExpr)
    (hsz : (a1.size == a2.size) = true)
    (hall : (a1.attach.zip a2).all (fun p => symExprEqB p.1.val p.2) = true)
    (ih : ∀ (p : { x // x ∈ a1 } × SymExpr), symExprEqB p.fst.val p.snd = true → p.fst.val = p.snd) :
    a1 = a2 := by
  have hs : a1.size = a2.size := by simpa using hsz
  apply Array.ext hs; intro i h1 h2
  have hz : i < (a1.attach.zip a2).size := by simp only [Array.size_zip, Array.size_attach]; omega
  have hb := Array.all_eq_true.mp hall i hz
  simp only [Array.getElem_zip, Array.getElem_attach] at hb
  exact ih (⟨a1[i], Array.getElem_mem h1⟩, a2[i]) hb

/-- SOUNDNESS of the reducible `symExprEqB` (Symbolic.lean): `symExprEqB a b = true → a = b`. UNCONDITIONAL
(it bottoms out at `valEqB`, which is `false` on any float, so a `true` result never involved a float ⇒
structural equality is propositional). This is the primitive the capstone's `.app`-IDENTITY IDEMPOTENCE case
(`x or x → x`) needs — the derived `SymExpr` BEq is `opaque`, so its `a == b` guard cannot be reduced in a
proof; `symExprEqB`'s can. Proven by `symExprEqB.induct`; the app/tuple/ctor array cases via `arrEqB_sound`,
ByteArray tags via `ByteArray.ext`, the `_ => false` shapes vacuously. -/
theorem symExprEqB_sound (a b : SymExpr) (h : symExprEqB a b = true) : a = b := by
  induction a, b using symExprEqB.induct with
  | case1 a b => simp only [symExprEqB, beq_iff_eq] at h; exact congrArg _ h
  | case2 a b => simp only [symExprEqB] at h; exact congrArg _ (valEqB_sound _ _ h)
  | case3 o1 a1 o2 a2 ih =>
      simp only [symExprEqB, Bool.and_eq_true, beq_iff_eq] at h
      obtain ⟨⟨ho, hsz⟩, hall⟩ := h; subst ho; rw [arrEqB_sound a1 a2 (by simpa using hsz) hall ih]
  | case4 c1 t1 e1 c2 t2 e2 ih3 ih2 ih1 =>
      simp only [symExprEqB, Bool.and_eq_true] at h
      obtain ⟨⟨hc, ht⟩, he⟩ := h; rw [ih3 hc, ih2 ht, ih1 he]
  | case5 a1 a2 ih =>
      simp only [symExprEqB, Bool.and_eq_true] at h
      obtain ⟨hsz, hall⟩ := h; rw [arrEqB_sound a1 a2 (by simpa using hsz) hall ih]
  | case6 t1 a1 t2 a2 ih =>
      simp only [symExprEqB, Bool.and_eq_true] at h
      obtain ⟨⟨ht, hsz⟩, hall⟩ := h
      have htb : t1 = t2 := ByteArray.ext (eq_of_beq ht); subst htb
      rw [arrEqB_sound a1 a2 (by simpa using hsz) hall ih]
  | case7 b1 s1 b2 s2 ih =>
      simp only [symExprEqB, Bool.and_eq_true] at h
      obtain ⟨hb, hs⟩ := h; rw [ih hb, ByteArray.ext (eq_of_beq hs)]
  | _ => simp [symExprEqB] at h

theorem normalizeAppIdentities_or_true_l (b : SymExpr) :
    normalizeAppIdentities "or" #[.const (.bool true), b] = .const (.bool true) := by
  simp [normalizeAppIdentities, isConstBool]
theorem normalizeAppIdentities_and_false_l (b : SymExpr) :
    normalizeAppIdentities "and" #[.const (.bool false), b] = .const (.bool false) := by
  simp [normalizeAppIdentities, isConstBool]
theorem normalizeAppIdentities_or_false_l (b : SymExpr) :
    normalizeAppIdentities "or" #[.const (.bool false), b] = b := by
  simp [normalizeAppIdentities, isConstBool]
theorem normalizeAppIdentities_and_true_l (b : SymExpr) :
    normalizeAppIdentities "and" #[.const (.bool true), b] = b := by
  simp [normalizeAppIdentities, isConstBool]

/-- CAPSTONE `.app`-IDENTITY (BOOL, DROP): `true or x → true`. `foldConst? "or"` fires on the `true` operand
regardless of `x` (`valIsBool` reduces it) → `v = .bool true`, matching `normalize`'s `.const (.bool true)`. -/
theorem denote_normalize_app_ident_or_true_l (ρ : Nat → Value) (w : IntTy) (a0 a1 : SymExpr) (v : Value)
    (hn0 : normalize a0 = .const (.bool true))
    (hnone : foldConst? "or" (#[a0, a1].attach.map (fun x => normalize x.val)) = none)
    (ih0 : ∀ u, denote ρ w a0 = .value u → denote ρ w (normalize a0) = .value u)
    (h : denote ρ w (.app "or" #[a0, a1]) = .value v) :
    denote ρ w (normalize (.app "or" #[a0, a1])) = .value v := by
  rw [normalize_app_ident "or" #[a0, a1] hnone,
      show (#[a0, a1] : Array SymExpr).attach.map (fun x => normalize x.val) = #[normalize a0, normalize a1] by simp,
      hn0, normalizeAppIdentities_or_true_l (normalize a1)]
  rw [denote_app2] at h
  obtain ⟨⟨u0, hu0⟩, ⟨u1, hu1⟩⟩ := denoteBinary_value_inv "or" w _ _ v h
  rw [hu0, hu1] at h
  have hu0' : u0 = .bool true := by
    have := ih0 u0 hu0; rw [hn0, denote_const_bool] at this; exact (Outcome.value.inj this).symm
  subst hu0'
  have hv : v = .bool true := by simp [denoteBinary, foldConst?, symToValue?, valIsBool] at h; exact h.symm
  rw [denote_const_bool, hv]

/-- CAPSTONE `.app`-IDENTITY (BOOL, DROP): `false and x → false`. -/
theorem denote_normalize_app_ident_and_false_l (ρ : Nat → Value) (w : IntTy) (a0 a1 : SymExpr) (v : Value)
    (hn0 : normalize a0 = .const (.bool false))
    (hnone : foldConst? "and" (#[a0, a1].attach.map (fun x => normalize x.val)) = none)
    (ih0 : ∀ u, denote ρ w a0 = .value u → denote ρ w (normalize a0) = .value u)
    (h : denote ρ w (.app "and" #[a0, a1]) = .value v) :
    denote ρ w (normalize (.app "and" #[a0, a1])) = .value v := by
  rw [normalize_app_ident "and" #[a0, a1] hnone,
      show (#[a0, a1] : Array SymExpr).attach.map (fun x => normalize x.val) = #[normalize a0, normalize a1] by simp,
      hn0, normalizeAppIdentities_and_false_l (normalize a1)]
  rw [denote_app2] at h
  obtain ⟨⟨u0, hu0⟩, ⟨u1, hu1⟩⟩ := denoteBinary_value_inv "and" w _ _ v h
  rw [hu0, hu1] at h
  have hu0' : u0 = .bool false := by
    have := ih0 u0 hu0; rw [hn0, denote_const_bool] at this; exact (Outcome.value.inj this).symm
  subst hu0'
  have hv : v = .bool false := by simp [denoteBinary, foldConst?, symToValue?, valIsBool] at h; exact h.symm
  rw [denote_const_bool, hv]

/-- `false or x` returns the surviving operand's value (which must be a bool for the app to be a value). The
bridge the PRESERVE bool cases need. Reduces the fold to a small `if`-over-`u1` form first (avoids the
`foldConst?`-unfold × Value-ctor blowup), then discharges each `u1` shape (bool splits on its literal). -/
theorem denoteBinary_or_false_l (w : IntTy) (u1 v : Value)
    (h : denoteBinary "or" w (.value (.bool false)) (.value u1) = .value v) : v = u1 := by
  simp only [denoteBinary, foldConst?] at h
  simp [symToValue?_const, valIsBool] at h
  cases u1 <;> simp_all <;> (rename_i b; cases b <;> simp_all)
/-- `true and x` companion of `denoteBinary_or_false_l`. -/
theorem denoteBinary_and_true_l (w : IntTy) (u1 v : Value)
    (h : denoteBinary "and" w (.value (.bool true)) (.value u1) = .value v) : v = u1 := by
  simp only [denoteBinary, foldConst?] at h
  simp [symToValue?_const, valIsBool] at h
  cases u1 <;> simp_all <;> (rename_i b; cases b <;> simp_all)

/-! ### BITWISE `.app`-identity cases are VACUOUS in `denote`. `denoteBinary` models only `arithOps`
(int `+ - * /`) on the deferred path; a BITWISE op (`& | ^ << >>`) is NOT in `arithOps` and `foldConst?`
has no bitwise arm, so `denoteBinary bitwise (.value _) (.value _)` is always `.unsupported` — never a value.
So in the top-level `denote.induct` `.app` case, a bitwise-op node with a value hypothesis is contradictory
⇒ the bitwise identity sub-cases (`x&0→0`, `x^x→0`, `x|0→x`, `x<<0→x`, `x>>0→x`) hold VACUOUSLY. Only the
symbol bitwise ops get identity rewrites in `normalizeAppIdentities` (word-forms `band`/… don't), so these
five suffice. Proof: `foldConst?` declines (concrete op string), then the deferred `.int,.int / _` match is
`.unsupported` either way (`arithOps.contains` is false). -/
theorem denoteBinary_amp_ne_value (w : IntTy) (va vb v : Value) :
    denoteBinary "&" w (.value va) (.value vb) ≠ .value v := by
  intro h
  have hfc : foldConst? "&" #[.const va, .const vb] = none := by simp [foldConst?, symToValue?, valIsBool]
  simp only [denoteBinary] at h; rw [hfc] at h; simp only [] at h; split at h <;> simp_all [arithOps]
theorem denoteBinary_bor_ne_value (w : IntTy) (va vb v : Value) :
    denoteBinary "|" w (.value va) (.value vb) ≠ .value v := by
  intro h
  have hfc : foldConst? "|" #[.const va, .const vb] = none := by simp [foldConst?, symToValue?, valIsBool]
  simp only [denoteBinary] at h; rw [hfc] at h; simp only [] at h; split at h <;> simp_all [arithOps]
theorem denoteBinary_bxor_ne_value (w : IntTy) (va vb v : Value) :
    denoteBinary "^" w (.value va) (.value vb) ≠ .value v := by
  intro h
  have hfc : foldConst? "^" #[.const va, .const vb] = none := by simp [foldConst?, symToValue?, valIsBool]
  simp only [denoteBinary] at h; rw [hfc] at h; simp only [] at h; split at h <;> simp_all [arithOps]
theorem denoteBinary_shl_ne_value (w : IntTy) (va vb v : Value) :
    denoteBinary "<<" w (.value va) (.value vb) ≠ .value v := by
  intro h
  have hfc : foldConst? "<<" #[.const va, .const vb] = none := by simp [foldConst?, symToValue?, valIsBool]
  simp only [denoteBinary] at h; rw [hfc] at h; simp only [] at h; split at h <;> simp_all [arithOps]
theorem denoteBinary_shr_ne_value (w : IntTy) (va vb v : Value) :
    denoteBinary ">>" w (.value va) (.value vb) ≠ .value v := by
  intro h
  have hfc : foldConst? ">>" #[.const va, .const vb] = none := by simp [foldConst?, symToValue?, valIsBool]
  simp only [denoteBinary] at h; rw [hfc] at h; simp only [] at h; split at h <;> simp_all [arithOps]

/-- CAPSTONE `.app`-IDENTITY (BOOL, PRESERVE): `false or x → x`. `foldConst? "or"` returns the surviving
operand (which is a bool); `ih1` carries `denote (normalize a1) = denote a1 = .value v`. -/
theorem denote_normalize_app_ident_or_false_l (ρ : Nat → Value) (w : IntTy) (a0 a1 : SymExpr) (v : Value)
    (hn0 : normalize a0 = .const (.bool false))
    (hnone : foldConst? "or" (#[a0, a1].attach.map (fun x => normalize x.val)) = none)
    (ih1 : ∀ u, denote ρ w a1 = .value u → denote ρ w (normalize a1) = .value u)
    (ih0 : ∀ u, denote ρ w a0 = .value u → denote ρ w (normalize a0) = .value u)
    (h : denote ρ w (.app "or" #[a0, a1]) = .value v) :
    denote ρ w (normalize (.app "or" #[a0, a1])) = .value v := by
  rw [normalize_app_ident "or" #[a0, a1] hnone,
      show (#[a0, a1] : Array SymExpr).attach.map (fun x => normalize x.val) = #[normalize a0, normalize a1] by simp,
      hn0, normalizeAppIdentities_or_false_l (normalize a1)]
  rw [denote_app2] at h
  obtain ⟨⟨u0, hu0⟩, ⟨u1, hu1⟩⟩ := denoteBinary_value_inv "or" w _ _ v h
  rw [hu0, hu1] at h
  have hu0' : u0 = .bool false := by
    have := ih0 u0 hu0; rw [hn0, denote_const_bool] at this; exact (Outcome.value.inj this).symm
  subst hu0'
  rw [ih1 u1 hu1, denoteBinary_or_false_l w u1 v h]

/-- CAPSTONE `.app`-IDENTITY (BOOL, PRESERVE): `true and x → x`. -/
theorem denote_normalize_app_ident_and_true_l (ρ : Nat → Value) (w : IntTy) (a0 a1 : SymExpr) (v : Value)
    (hn0 : normalize a0 = .const (.bool true))
    (hnone : foldConst? "and" (#[a0, a1].attach.map (fun x => normalize x.val)) = none)
    (ih1 : ∀ u, denote ρ w a1 = .value u → denote ρ w (normalize a1) = .value u)
    (ih0 : ∀ u, denote ρ w a0 = .value u → denote ρ w (normalize a0) = .value u)
    (h : denote ρ w (.app "and" #[a0, a1]) = .value v) :
    denote ρ w (normalize (.app "and" #[a0, a1])) = .value v := by
  rw [normalize_app_ident "and" #[a0, a1] hnone,
      show (#[a0, a1] : Array SymExpr).attach.map (fun x => normalize x.val) = #[normalize a0, normalize a1] by simp,
      hn0, normalizeAppIdentities_and_true_l (normalize a1)]
  rw [denote_app2] at h
  obtain ⟨⟨u0, hu0⟩, ⟨u1, hu1⟩⟩ := denoteBinary_value_inv "and" w _ _ v h
  rw [hu0, hu1] at h
  have hu0' : u0 = .bool true := by
    have := ih0 u0 hu0; rw [hn0, denote_const_bool] at this; exact (Outcome.value.inj this).symm
  subst hu0'
  rw [ih1 u1 hu1, denoteBinary_and_true_l w u1 v h]

/-- CAPSTONE tuple case (full-equality, per-element IH): `denote` MODELS `.tuple` (each element folded
through `outcomeToValue`), so `denote (normalize (.tuple es)) = denote (.tuple es)` needs the per-element
congruence `denote (normalize eᵢ) = denote eᵢ` (the IH the eventual `denote.induct` supplies). This is the
Array `.attach.map` congruence that was banked; proven here by extensionality over the element index. -/
theorem denote_normalize_tuple (ρ : Nat → Value) (w : IntTy) (es : Array SymExpr)
    (ih : ∀ x ∈ es, denote ρ w (normalize x) = denote ρ w x) :
    denote ρ w (normalize (.tuple es)) = denote ρ w (.tuple es) := by
  rw [normalize_tuple, denote_tuple, denote_tuple]
  congr 2
  apply Array.ext
  · simp
  · intro i h1 h2
    simp only [Array.getElem_map, Array.getElem_attach]
    rw [ih _ (Array.getElem_mem _)]

/-- Shared Array `.attach.map` congruence used by the capstone `.app` case (and the same shape as the
tuple/record element congruence): if each arg's denotation is congruence-invariant under `normalize`
(`denote (normalize aᵢ) = denote aᵢ`), the whole `denote`-mapped arg array is unchanged. `denoteApp`
(which `denote (.app …)` calls on this map) then commutes with `normalize` on the arguments. -/
theorem denote_map_normalize_args (ρ : Nat → Value) (w : IntTy) (args : Array SymExpr)
    (ih : ∀ a ∈ args, denote ρ w (normalize a) = denote ρ w a) :
    args.attach.map (fun x => denote ρ w (normalize x.val))
      = args.attach.map (fun x => denote ρ w x.val) := by
  apply Array.ext
  · simp
  · intro i h1 h2
    simp only [Array.getElem_map, Array.getElem_attach]
    rw [ih _ (Array.getElem_mem _)]

/-- CAPSTONE record case (full-equality, per-element IH over the field VALUES). Same Array `.attach.map`
congruence as the tuple case, over field pairs `(key, value)` — the key is preserved, the value normalized. -/
theorem denote_normalize_record (ρ : Nat → Value) (w : IntTy) (fs : Array (ByteArray × SymExpr))
    (ih : ∀ x ∈ fs, denote ρ w (normalize x.2) = denote ρ w x.2) :
    denote ρ w (normalize (.record fs)) = denote ρ w (.record fs) := by
  rw [normalize_record, denote_record, denote_record]
  congr 2
  apply Array.ext
  · simp
  · intro i h1 h2
    simp only [Array.getElem_map, Array.getElem_attach]
    rw [ih _ (Array.getElem_mem _)]

/-! ### Capstone `.ite` building blocks: denote-soundness of the boolean-materialization identities.
`normalize` rewrites `if c then true else false → c` and `if c then false else true → not c` (#6450).
These pin the denote-soundness of that rewrite in VALUE-CONDITIONED form: when the condition denotes to
a boolean `b`, the materializing `ite` denotes to exactly `b` (resp. `!b`) — i.e. `denote` agrees with
the rewritten `c` (resp. `not c`). The `.value (.bool b)` hypothesis is what the capstone `.ite` case
supplies from the IH on the condition; a non-bool `c` never reaches this branch (the rewrite fires only
when both branches are bool literals, and an ill-typed `c` never compiles into the differential). -/
theorem denote_ite_materialize_true (ρ : Nat → Value) (w : IntTy) (c : SymExpr) (b : Bool)
    (h : denote ρ w c = .value (.bool b)) :
    denote ρ w (.ite c (.const (.bool true)) (.const (.bool false))) = .value (.bool b) := by
  simp only [denote, h, Value.asF64?]
  cases b <;> rfl

theorem denote_ite_materialize_false (ρ : Nat → Value) (w : IntTy) (c : SymExpr) (b : Bool)
    (h : denote ρ w c = .value (.bool b)) :
    denote ρ w (.ite c (.const (.bool false)) (.const (.bool true))) = .value (.bool (!b)) := by
  simp only [denote, h, Value.asF64?]
  cases b <;> rfl


/-- denote-soundness of `not`: when its operand denotes to a boolean `b`, `(not c)` denotes to `!b`.
The denote side of the `if c then false else true → not c` materialization (#6450) — the capstone `.ite`
materialize-false sub-case needs this. Reduces now that `foldConst?`'s `not` is a leading size-dispatch
(the old array-literal `not` arm walled `simp`); `symToValue?_const` discharges the const extraction. -/
theorem denote_not_bool (ρ : Nat → Value) (w : IntTy) (c : SymExpr) (b : Bool)
    (h : denote ρ w c = .value (.bool b)) :
    denote ρ w (.app "not" #[c]) = .value (.bool (!b)) := by
  cases b <;> simp [denote, denoteApp, denoteUnary, foldConst?, symToValue?_const, h]

/-- SEMANTIC EQUIVALENCE of the `if c then false else true → not c` materialization (#6450), on the
value-producing valuations: both sides denote to `!b` when the condition denotes to a boolean `b`. This
is exactly the fact the capstone `.ite` materialize-false sub-case discharges — `normalize` rewrites the
`ite` to `not c'`, and this shows `denote` is unchanged. Composes `denote_ite_materialize_false` (#6467,
the ite side) with `denote_not_bool` (the `not` side, now that `foldConst? "not"` reduces). -/
theorem denote_ite_materialize_false_eq_not (ρ : Nat → Value) (w : IntTy) (c : SymExpr) (b : Bool)
    (h : denote ρ w c = .value (.bool b)) :
    denote ρ w (.ite c (.const (.bool false)) (.const (.bool true))) = denote ρ w (.app "not" #[c]) := by
  rw [denote_ite_materialize_false ρ w c b h, denote_not_bool ρ w c b h]

/-- INVERSION for the `.ite` case: an `ite` denotes to a VALUE only via the branch its condition selects
— the condition denotes to a boolean, and the taken branch denotes to that same value. (`denote`'s `.ite`
arm returns a `.value` only in the `bool true`/`bool false` condition sub-cases; a non-bool value gives
`.unsupported`, a trap/divergence propagates.) This is the case-analysis foundation of the eventual
capstone `.ite` induction step: it turns a value-producing `ite` into a fact about the selected branch,
which the branch IH then transports across `normalize`. -/
theorem denote_ite_value_inv (ρ : Nat → Value) (w : IntTy) (c t e : SymExpr) (v : Value)
    (h : denote ρ w (.ite c t e) = .value v) :
    (denote ρ w c = .value (.bool true) ∧ denote ρ w t = .value v)
      ∨ (denote ρ w c = .value (.bool false) ∧ denote ρ w e = .value v) := by
  simp only [denote] at h
  split at h <;>
    first
    | exact Or.inl ⟨‹_›, h⟩
    | exact Or.inr ⟨‹_›, h⟩
    | simp_all

/-! ### Capstone `.ite` fold-SELECT step (the sub-case where the condition folds to a bool literal).
When `normalize c` folds to `.const (.bool true/false)`, the whole `ite` normalizes to the selected
branch (`normalize_ite_condTrue/False`, #6472). These lemmas prove that sub-case of the value-conditioned
`.ite` induction step: given the branch IH (`iht`/`ihe`) and the condition IH (`ihc`), a value-producing
`ite` transports across `normalize`. The condition IH is what rules out the *other* branch — if the
`ite` actually took the false branch while `normalize c` folded to `true`, `ihc` would force
`denote (normalize c) = .value (.bool false)`, contradicting `denote (.const (.bool true)) = .value
(.bool true)`. No `foldConst?`/complex-arm exposure — this is the clean structural half of the step. -/
theorem denote_normalize_ite_condTrue_step (ρ : Nat → Value) (w : IntTy) (c t e : SymExpr)
    (ihc : ∀ u, denote ρ w c = .value u → denote ρ w (normalize c) = .value u)
    (iht : ∀ u, denote ρ w t = .value u → denote ρ w (normalize t) = .value u)
    (hnc : normalize c = .const (.bool true)) (v : Value)
    (h : denote ρ w (.ite c t e) = .value v) :
    denote ρ w (normalize (.ite c t e)) = .value v := by
  rw [normalize_ite_condTrue c t e hnc]
  rcases denote_ite_value_inv ρ w c t e v h with ⟨_, ht⟩ | ⟨hc, _⟩
  · exact iht v ht
  · have := ihc _ hc
    rw [hnc] at this
    simp [denote, Value.asF64?] at this

theorem denote_normalize_ite_condFalse_step (ρ : Nat → Value) (w : IntTy) (c t e : SymExpr)
    (ihc : ∀ u, denote ρ w c = .value u → denote ρ w (normalize c) = .value u)
    (ihe : ∀ u, denote ρ w e = .value u → denote ρ w (normalize e) = .value u)
    (hnc : normalize c = .const (.bool false)) (v : Value)
    (h : denote ρ w (.ite c t e) = .value v) :
    denote ρ w (normalize (.ite c t e)) = .value v := by
  rw [normalize_ite_condFalse c t e hnc]
  rcases denote_ite_value_inv ρ w c t e v h with ⟨hc, _⟩ | ⟨_, he⟩
  · have := ihc _ hc
    rw [hnc] at this
    simp [denote, Value.asF64?] at this
  · exact ihe v he

/-- Capstone `.ite` MATERIALIZE-true STEP (`if c then true else false → c`, the `c'`-rebuild sub-case):
given the condition/branch IHs and the branches normalizing to `true`/`false`, a value-producing `ite`
transports across `normalize` (which collapses it to `normalize c`, #6513). Whichever branch the `ite`
took, the produced value equals the (normalized) condition's: the taken branch is a bool literal fixing
`v`, and `ihc` carries `denote c` to `denote (normalize c) = .value v`. -/
theorem denote_normalize_ite_materializeTrue_step (ρ : Nat → Value) (w : IntTy) (c t e : SymExpr)
    (ihc : ∀ u, denote ρ w c = .value u → denote ρ w (normalize c) = .value u)
    (iht : ∀ u, denote ρ w t = .value u → denote ρ w (normalize t) = .value u)
    (ihe : ∀ u, denote ρ w e = .value u → denote ρ w (normalize e) = .value u)
    (hnt : normalize t = .const (.bool true)) (hne : normalize e = .const (.bool false))
    (v : Value) (h : denote ρ w (.ite c t e) = .value v) :
    denote ρ w (normalize (.ite c t e)) = .value v := by
  rw [normalize_ite_materializeTrue c t e hnt hne]
  rcases denote_ite_value_inv ρ w c t e v h with ⟨hc, ht⟩ | ⟨hc, he⟩
  · have hvt := iht _ ht
    rw [hnt] at hvt
    simp only [denote, Value.asF64?] at hvt
    rw [ihc _ hc]; exact hvt
  · have hve := ihe _ he
    rw [hne] at hve
    simp only [denote, Value.asF64?] at hve
    rw [ihc _ hc]; exact hve

/-- denote-side of the equal-branch COLLAPSE identity (`if c then a else a → a`): whichever branch the
condition selects, the value is `a`'s — so a value-producing `ite` with identical branches denotes to
exactly what that branch does. This is the denote half of the capstone `.ite` collapse sub-case (the
value-conditioned formulation needs no `!mayTrap` guard here: `denote (.ite c a a) = .value v` already
witnesses that the condition neither trapped nor diverged in this valuation). -/
theorem denote_ite_same_value (ρ : Nat → Value) (w : IntTy) (c a : SymExpr) (v : Value)
    (h : denote ρ w (.ite c a a) = .value v) : denote ρ w a = .value v := by
  rcases denote_ite_value_inv ρ w c a a v h with ⟨_, ha⟩ | ⟨_, ha⟩ <;> exact ha

/-- `symToValue?` IDEMPOTENCE: extracting a value and re-wrapping it as a `.const` round-trips —
`symToValue? (.const v) = some v` for the `v` any expression extracts to. (A direct corollary of
`symToValue?_const`; pins that the folder's constant-extraction is stable under re-constification, an
invariant the fold/normalize pipeline relies on when it replaces a folded sub-term by `.const v`.) -/
theorem symToValue?_idem (e : SymExpr) (v : Value) (h : symToValue? e = some v) :
    symToValue? (.const v) = some v := symToValue?_const v

/-- `normalize`-`ite` MATERIALIZE-false structural equation, in the `c'`-rebuild sub-case: when the
condition does NOT fold to a bool literal (the `≠` hypotheses put us in the rebuild arm) and the branches
normalize to `false`/`true`, the `ite` normalizes to `not (normalize c)`. (Unlike materialize-TRUE, this
needs the `≠ const bool` hypotheses: on a const-fold condition the arm would return `normalize t/e`, not
`not (normalize c)`.) -/
theorem normalize_ite_materializeFalse (c t e : SymExpr)
    (h1 : normalize c ≠ .const (.bool true)) (h2 : normalize c ≠ .const (.bool false))
    (hnt : normalize t = .const (.bool false)) (hne : normalize e = .const (.bool true)) :
    normalize (.ite c t e) = .app "not" #[normalize c] := by
  simp only [normalize, hnt, hne]

/-- Capstone `.ite` MATERIALIZE-false STEP (`if c then false else true → not c`). Uses the structural
equation above + the condition IH to pin `denote (not (normalize c)) = .value v`: `denote_not_bool`
turns it into `!b` where `denote (normalize c) = .value (.bool b)` (via `ihc`), and the taken bool-literal
branch fixes `v = !b`. -/
theorem denote_normalize_ite_materializeFalse_step (ρ : Nat → Value) (w : IntTy) (c t e : SymExpr)
    (ihc : ∀ u, denote ρ w c = .value u → denote ρ w (normalize c) = .value u)
    (iht : ∀ u, denote ρ w t = .value u → denote ρ w (normalize t) = .value u)
    (ihe : ∀ u, denote ρ w e = .value u → denote ρ w (normalize e) = .value u)
    (h1 : normalize c ≠ .const (.bool true)) (h2 : normalize c ≠ .const (.bool false))
    (hnt : normalize t = .const (.bool false)) (hne : normalize e = .const (.bool true))
    (v : Value) (h : denote ρ w (.ite c t e) = .value v) :
    denote ρ w (normalize (.ite c t e)) = .value v := by
  rw [normalize_ite_materializeFalse c t e h1 h2 hnt hne]
  rcases denote_ite_value_inv ρ w c t e v h with ⟨hc, ht⟩ | ⟨hc, he⟩
  · rw [denote_not_bool ρ w (normalize c) true (ihc _ hc)]
    have hvt := iht _ ht; rw [hnt] at hvt
    simp only [denote, Value.asF64?] at hvt; simpa using hvt
  · rw [denote_not_bool ρ w (normalize c) false (ihc _ hc)]
    have hve := ihe _ he; rw [hne] at hve
    simp only [denote, Value.asF64?] at hve; simpa using hve

/-- Capstone `.ite` PLAIN-rebuild STEP (the fall-through case: no const-fold, no materialize, no
collapse — `normalize` rebuilds `.ite (normalize c) (normalize t) (normalize e)`). Given that plain
shape (`hplain`, discharged by the eventual assembly when it establishes the fall-through fires) and the
IHs, the value-producing `ite` transports across `normalize` directly via `denote`'s own `.ite` arm: the
condition IH pins `denote (normalize c)` to the selecting boolean, and the taken-branch IH pins its value.
No `BEq`-agreement needed (unlike collapse) — this is the clean fall-through half. -/
theorem denote_normalize_ite_plain_step (ρ : Nat → Value) (w : IntTy) (c t e : SymExpr)
    (ihc : ∀ u, denote ρ w c = .value u → denote ρ w (normalize c) = .value u)
    (iht : ∀ u, denote ρ w t = .value u → denote ρ w (normalize t) = .value u)
    (ihe : ∀ u, denote ρ w e = .value u → denote ρ w (normalize e) = .value u)
    (hplain : normalize (.ite c t e) = .ite (normalize c) (normalize t) (normalize e))
    (v : Value) (h : denote ρ w (.ite c t e) = .value v) :
    denote ρ w (normalize (.ite c t e)) = .value v := by
  rw [hplain]
  rcases denote_ite_value_inv ρ w c t e v h with ⟨hc, ht⟩ | ⟨hc, he⟩
  · simp only [denote, ihc _ hc, iht _ ht]
  · simp only [denote, ihc _ hc, ihe _ he]

/-- Capstone `.ite` equal-branch COLLAPSE STEP (`if c then t else e` with `t' == e' → t'`). Given the
collapse shape (`hcollapse`, discharged when the guard fires) and the BRANCH-AGREEMENT
`denote (normalize t) = denote (normalize e)` (`hagree` — the semantic content of `t' == e'` on the
FLOAT-FREE branches the guard requires; see #6533), a value-producing `ite` transports across `normalize`
regardless of which branch it took. This completes the FIFTH and last `.ite` sub-case step; the ONLY
remaining capstone `.ite` gap is discharging `hagree` from the float-free `BEq` (a conditional-`LawfulBEq`
Array induction — the isolated obstacle). Note: NO `!mayTrap` needed here — `denote (ite c t e) = .value v`
already witnesses the condition did not trap/diverge in this valuation. -/
theorem denote_normalize_ite_collapse_step (ρ : Nat → Value) (w : IntTy) (c t e : SymExpr)
    (iht : ∀ u, denote ρ w t = .value u → denote ρ w (normalize t) = .value u)
    (ihe : ∀ u, denote ρ w e = .value u → denote ρ w (normalize e) = .value u)
    (hcollapse : normalize (.ite c t e) = normalize t)
    (hagree : denote ρ w (normalize t) = denote ρ w (normalize e))
    (v : Value) (h : denote ρ w (.ite c t e) = .value v) :
    denote ρ w (normalize (.ite c t e)) = .value v := by
  rw [hcollapse]
  rcases denote_ite_value_inv ρ w c t e v h with ⟨_, ht⟩ | ⟨_, he⟩
  · exact iht v ht
  · rw [hagree]; exact ihe v he

end Oracle
