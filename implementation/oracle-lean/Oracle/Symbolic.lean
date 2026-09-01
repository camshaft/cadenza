/-
`Oracle.Symbolic` — the T2 SYMBOLIC-EQUIVALENCE arm (operator seq-196, ruling B): prove an input program
`P` and its `--target cadenza` round-trip `P' = roundtrip(P)` are functionally equivalent FOR ALL INPUTS,
by symbolic evaluation → canonical normalization → structural equality (NOT sampled). A fresh symbolic
VARIABLE stands for each program parameter (the whole input domain); `symEval` evaluates a program body to
a `SymExpr` over those vars; `normalize` canonicalizes; two programs whose normal forms are structurally
equal are PROVEN equivalent for all inputs. When it cannot decide (an unmodeled construct, or two
fully-normalized-but-different forms) it returns `cannotProve` — it NEVER claims a false divergence (the
normalizer is deliberately incomplete; a genuine divergence is confirmed by the sampled differential).

This is the analyzable-fragment guarantee: straight-line scalar arithmetic/comparison/boolean + `if` are
provable here; `let`, match/sum, collections, and recursion are the incompleteness boundary (→ `cannotProve`,
degrade to v-cdz-smith's sampled net) and land in later increments (T2.0b+). SOUNDNESS RULE: every
`normalize` rewrite MUST be a semantic identity for all inputs INCLUDING trap behavior — so this first cut
does NOT fold arithmetic (folding needs width/overflow-trap-aware semantics; a wrong fold = a FALSE
"proven", the one outcome worse than `cannotProve`) and does NOT reassociate `+`/`*` (can move the overflow
trap point). It only performs unconditionally-sound rewrites: `if` on a constant condition selects the
branch, and an `if` with identical branches collapses.
-/
import Oracle.Ast
import Oracle.Value
import Oracle.Eval

namespace Oracle
-- NB: do NOT `open Oracle.Value` — its `some`/`none`/`ok`/`err` constructors would make bare `some`/`none`
-- (Option, used pervasively below) ambiguous. `Value` is in scope (same namespace); `Value.ofLeaf` is
-- qualified; and Value scalars are built via `.int`/`.bool`/`.unit` anonymous-constructor notation.
open Oracle.Ast Eval

/-- A SYMBOLIC value: an expression over symbolic input variables (`var n` = the n-th program parameter)
and concrete constants, plus the modeled scalar operators and conditionals. -/
inductive SymExpr where
  | var (n : Nat)                            -- the n-th program parameter (a symbolic input)
  | const (v : Value)                        -- a concrete value (a literal, or a future folded constant)
  | app (op : String) (args : Array SymExpr) -- a modeled operator (`+ - * / % < > <= >= = and or not`)
  | ite (c t e : SymExpr)                    -- a conditional on a (possibly symbolic) condition
  | tuple (elems : Array SymExpr)            -- a tuple value (lazy elements); positional projection reads one
  | record (fields : Array (ByteArray × SymExpr)) -- a record value, fields sorted by key; field projection reads one
  | ctor (tag : ByteArray) (args : Array SymExpr)  -- a tagged constructor value (Some/Ok/Err payload, None nullary)
  -- SYMBOLIC destructuring (scrutinee not statically a concrete ctor/tuple): `proj` = the payload/element a
  -- pattern binds (`sel` = ctor name for a variant payload, or a decimal index for a tuple element); `case`
  -- = a match over a symbolic scrutinee, arms kept ORDER-SENSITIVELY as (discriminant-tag, body). Two `case`s
  -- (or `proj`s) are equal iff structurally equal — so P and its cadenza round-trip with the SAME match prove.
  | proj (base : SymExpr) (sel : ByteArray)
  | case (scrut : SymExpr) (arms : Array (ByteArray × SymExpr))
  -- a LOCAL FUNCTION bound in a `(do …)` (a `(def (f params) body)`): carries its param-spec node ids, body
  -- node id, and the CAPTURED def-site env (`cap`), so a later `(f args)` call INLINES it in `params ++ cap`
  -- (see `symDo`/the call path) — modeling a closure over enclosing do-bindings. `cap` uses the raw
  -- `List (ByteArray × SymExpr)` type since `SymEnv` is defined below. Inert under normalize; BEq structural.
  | localFn (specs : Array Nat) (bodyId : Nat) (cap : List (ByteArray × SymExpr))
  deriving BEq, Inhabited

/-- The symbolic OUTCOME of evaluating a program with symbolic inputs. `cannotProve` records WHY so the
caller can distinguish an incompleteness-boundary gap from a strong (normalized-but-different) lead. -/
inductive SymOutcome where
  | sym (e : SymExpr)
  | cannotProve (reason : String)
  deriving BEq, Inhabited

/-- A fully-CONSTANT symbolic expression → its concrete `Value` (`none` if any leaf is symbolic). Lets the
constant folder decide `=` over COMPOUND constants (e.g. `(= (tuple 1 2) (tuple 1 3))`), not just scalars. -/
def symToValue? : SymExpr → Option Value
  | .const v => some v
  | .tuple es => (es.attach.mapM (fun x => symToValue? x.val)).map Value.tuple
  | .record fs => (fs.attach.mapM (fun x => (symToValue? x.val.2).map (fun v => (x.val.1, v)))).map Value.record
  | _ => none
termination_by e => sizeOf e
decreasing_by
  · simp_wf; have h := Array.sizeOf_lt_of_mem x.property; omega
  · simp_wf
    rcases x with ⟨⟨k, e⟩, hmem⟩
    have h := Array.sizeOf_lt_of_mem hmem
    simp_all
    omega

/-- Convert a fully-CONCRETE element expression to a `Value`, including compound `.ctor "list"` (order +
duplicates preserved — faithful to `Value.list`), `.tuple`, and `.record`. Used ONLY by `Set.len`/`Map.len`
canonicalization to count DISTINCT compound elements (via eval's `canonSet`/`canonMap`); kept SEPARATE from
the capstone-critical `symToValue?` (which the `denote (normalize e) = denote e` proofs case on and must
stay minimal — const/tuple/record only). A non-list `.ctor` (set/map as an element) or any symbolic subterm
→ `none` (the caller degrades to `cannotProve`, a sound skip). -/
def symElemToValue? : SymExpr → Option Value
  | .const v => some v
  | .tuple es => (es.attach.mapM (fun x => symElemToValue? x.val)).map Value.tuple
  | .record fs => (fs.attach.mapM (fun x => (symElemToValue? x.val.2).map (fun v => (x.val.1, v)))).map Value.record
  | .ctor name args =>
      if name == "list".toUTF8 then (args.attach.mapM (fun x => symElemToValue? x.val)).map Value.list
      else none
  | _ => none
termination_by e => sizeOf e
decreasing_by
  · simp_wf; have h := Array.sizeOf_lt_of_mem x.property; omega
  · simp_wf; rcases x with ⟨⟨k, e⟩, hmem⟩; have h := Array.sizeOf_lt_of_mem hmem; simp_all; omega
  · simp_wf; have h := Array.sizeOf_lt_of_mem x.property; omega

/-- Convert a `Value` back to the SymExpr representation `symEval` uses — the inverse of `symElemToValue?`
over its output range: a scalar → `.const`, `.tuple`/`.record` → the same-shaped SymExpr (children
converted), a `.list` → `.ctor "list"` (matching how `symEval` builds list literals). Used to REBUILD a
canonicalized collection (e.g. `Set.insert`'s `canonSet` output) as a SymExpr value; representation-faithful
so a rebuilt set matches a set built from a literal of the same elements. -/
def valueToSym : Value → SymExpr
  | .tuple es => .tuple (es.attach.map (fun x => valueToSym x.val))
  | .record fs => .record (fs.attach.map (fun x => (x.val.1, valueToSym x.val.2)))
  | .list es => .ctor "list".toUTF8 (es.attach.map (fun x => valueToSym x.val))
  | v => .const v
termination_by v => sizeOf v
decreasing_by
  · simp_wf; have h := Array.sizeOf_lt_of_mem x.property; omega
  · simp_wf; rcases x with ⟨⟨k, e⟩, hmem⟩; have h := Array.sizeOf_lt_of_mem hmem; simp_all; omega
  · simp_wf; have h := Array.sizeOf_lt_of_mem x.property; omega

/-- A REDUCIBLE STRUCTURAL equality on `Value` for the scalar + single-payload fragment (int/bool/str/char/
bytes/rational/unit/none, and some/ok/err/variant recursively). Returns `false` on any FLOAT ctor
(`float`/`floatNan`/`floatInf`/`f64`) and on the collection ctors (`tuple`/`list`/`set`/`map`/`record`).
Foundation of the reducible SymExpr equality the capstone's `.app`-IDENTITY IDEMPOTENCE case needs (the
derived `Value`/`SymExpr` `BEq` is `opaque` — kernel-irreducible — so a proof cannot reduce `a == b`).
The `false`-on-float design makes soundness UNCONDITIONAL: `valEqB a b = true` can only hold when NO float
is involved, hence structural equality ⇒ propositional equality (`valEqB_sound`). ByteArray fields compare
via `.data` (`Array UInt8` IS `LawfulBEq`, unlike `ByteArray`). Collections `false` = sound but incomplete
(a first increment; compound-collection equality is a follow-up). -/
def valEqB : Value → Value → Bool
  | .int a, .int b => a == b
  | .bool a, .bool b => a == b
  | .str a, .str b => a.data == b.data
  | .char a, .char b => a.data == b.data
  | .bytes a, .bytes b => a.data == b.data
  | .rational a b, .rational c d => a == c && b == d
  | .unit, .unit => true
  | .none, .none => true
  | .some a, .some b => valEqB a b
  | .ok a, .ok b => valEqB a b
  | .err a, .err b => valEqB a b
  | .variant t1 p1, .variant t2 p2 => t1.data == t2.data && valEqB p1 p2
  | _, _ => false

/-- A REDUCIBLE STRUCTURAL equality on `SymExpr` (uses `valEqB` at `.const`, so `false` on any float leaf →
soundness is unconditional: `= true` implies no float, hence propositional equality). The reducible
counterpart of the OPAQUE derived `SymExpr` `BEq` — needed to reason about the `.app`-IDENTITY IDEMPOTENCE
guard (`x or x → x` fires on `a == b`, which a proof cannot reduce). `.record`/`.case`/`.localFn` (pair-array
/ list shapes) → `false` for now (sound-but-incomplete; a later increment). Byte strings compare via `.data`
(`Array UInt8` IS `LawfulBEq`; `ByteArray` is not). Array cases via `.attach.zip` so each element carries a
membership proof (`Array.sizeOf_lt_of_mem`) for termination. -/
def symExprEqB : SymExpr → SymExpr → Bool
  | .var a, .var b => a == b
  | .const a, .const b => valEqB a b
  | .app o1 a1, .app o2 a2 =>
      o1 == o2 && a1.size == a2.size && (a1.attach.zip a2).all (fun p => symExprEqB p.1.val p.2)
  | .ite c1 t1 e1, .ite c2 t2 e2 => symExprEqB c1 c2 && symExprEqB t1 t2 && symExprEqB e1 e2
  | .tuple a1, .tuple a2 =>
      a1.size == a2.size && (a1.attach.zip a2).all (fun p => symExprEqB p.1.val p.2)
  | .ctor t1 a1, .ctor t2 a2 =>
      t1.data == t2.data && a1.size == a2.size && (a1.attach.zip a2).all (fun p => symExprEqB p.1.val p.2)
  | .proj b1 s1, .proj b2 s2 => symExprEqB b1 b2 && s1.data == s2.data
  | _, _ => false
termination_by e _ => sizeOf e
decreasing_by
  all_goals simp_wf
  all_goals first
    | omega
    | (have h := Array.sizeOf_lt_of_mem p.1.property; omega)

/-- Is the VALUE `v` the boolean literal `b`? A REDUCIBLE STRUCTURAL check (`match` + `Bool` `==`), unlike
`v == .bool b` whose `Value` derived `BEq` is `opaque` (kernel-irreducible). Behavior-identical to
`v == .bool b`. Lets `foldConst?`'s `and`/`or` arm REDUCE in proofs (the capstone bool-identity cases need
to compute `foldConst? "or" #[.const (.bool true), …]`, which the opaque `==` blocked). Mirrors `isConstInt`
(the same fix for `SymExpr`-level guards, #7216). -/
def valIsBool (v : Value) (b : Bool) : Bool := match v with | .bool c => c == b | _ => false

/-- Constant-fold an operator applied to fully-CONSTANT operands, iff the fold is SOUND independent of
integer width (the symbolic evaluator does not yet track width). So this folds ONLY operators that can
never overflow / trap: COMPARISONS over integer constants (`< > <= >=`, total on `Int`), value EQUALITY
(`=`), and BOOLEAN ops (`and`/`or`/`not`) over boolean constants. Returns `none` (leave symbolic) for a
non-constant operand, a non-integer comparison, or ARITHMETIC (`+ - * / %`) — arithmetic folding needs
width/overflow-trap-aware semantics (T2.0d); folding it here could produce a value where the program
traps = a FALSE "proven", the one outcome worse than `cannotProve`. -/
def foldConst? (op : String) (args : Array SymExpr) : Option Value :=
  let consts := args.map symToValue?
  if consts.any (·.isNone) then none
  else
  -- UNARY `not` handled via a leading size-dispatch (NOT an array-literal pattern) so `foldConst? "not"`
  -- REDUCES cleanly in proofs — the `#[…]` literal patterns below compile to a `_sparseCasesOn` matcher
  -- that `simp`/`split` cannot reduce (same blocker the `denoteApp` size-dispatch refactor dodged). This
  -- is behavior-identical to the former `| "not", #[.bool b] => some (.bool (!b))` arm (a `not` over one
  -- boolean folds; any other `not` shape falls through to `none`, exactly as before).
  if op == "not" && (consts.filterMap id).size == 1 then
    (match (consts.filterMap id)[0]? with | some (Value.bool b) => some (.bool (!b)) | _ => none)
  -- BINARY ops via a `size == 2` SIZE-DISPATCH + `if op == …` chain (NOT `#[a,b]` array-literal patterns):
  -- byte-identical to the former `match op, #[a,b]` arms, but built only from splittable matches (string-eq
  -- `if`s, `Value`/`Option` matches) so the def REDUCES in proofs — the `#[…]` literal arms compiled to a
  -- `_sparseCasesOn` (`foldConst?.match_10`) with no equation lemmas, blocking `split`/`simp` (needed for
  -- `denote (normalize e) = denote e`'s `.app` fold-soundness / output-canon-stability). Same `not`-dispatch
  -- rationale (#6487). Int `<>≤≥` fold first (int-pattern), then FLOAT via `asF64?`; int `+-*/` stay
  -- symbolic (width/trap-deferred). All fold outputs are `.bool`/`.f64` (never `.int`) — hence canon-stable.
  else if (consts.filterMap id).size == 2 then
    let a := (consts.filterMap id)[0]!
    let b := (consts.filterMap id)[1]!
    if op == "=" then some (.bool (Value.valueEqSpec a b))
    -- ORDERING (`< > <= >=`): int fast-path, then the type's THREE-WAY total order (`compareVals` +
    -- `cmpHolds` — bool/String/Char/Bytes/Rational, matching `evalCmp`), then FLOAT via `asF64?` (IEEE
    -- partial order; `compareVals` returns `none` on floats — no total order). Byte-identical to `evalCmp`.
    else if op == "<" then (match a, b with
                            | .int x, .int y => some (.bool (decide (x < y)))
                            | _, _ => (match compareVals a b with
                                       | some o => some (.bool (cmpHolds "<" o))
                                       | none => (match Value.asF64? a, Value.asF64? b with | some x, some y => some (.bool (x < y)) | _, _ => none)))
    else if op == ">" then (match a, b with
                            | .int x, .int y => some (.bool (decide (x > y)))
                            | _, _ => (match compareVals a b with
                                       | some o => some (.bool (cmpHolds ">" o))
                                       | none => (match Value.asF64? a, Value.asF64? b with | some x, some y => some (.bool (x > y)) | _, _ => none)))
    else if op == "<=" then (match a, b with
                             | .int x, .int y => some (.bool (decide (x ≤ y)))
                             | _, _ => (match compareVals a b with
                                        | some o => some (.bool (cmpHolds "<=" o))
                                        | none => (match Value.asF64? a, Value.asF64? b with | some x, some y => some (.bool (x ≤ y)) | _, _ => none)))
    else if op == ">=" then (match a, b with
                             | .int x, .int y => some (.bool (decide (x ≥ y)))
                             | _, _ => (match compareVals a b with
                                        | some o => some (.bool (cmpHolds ">=" o))
                                        | none => (match Value.asF64? a, Value.asF64? b with | some x, some y => some (.bool (x ≥ y)) | _, _ => none)))
    else if op == "+" || op == "-" || op == "*" || op == "/" then
      (match Value.asF64? a, Value.asF64? b with | some x, some y => (match evalFloatOp op x y with | .value v => some v | _ => none) | _, _ => none)
    else if op == "and" then
      (if valIsBool a true && valIsBool b true then some (.bool true)
       else if valIsBool a false || valIsBool b false then some (.bool false) else none)
    else if op == "or" then
      (if valIsBool a true || valIsBool b true then some (.bool true)
       else if valIsBool a false && valIsBool b false then some (.bool false) else none)
    else none
  -- non-binary `and`/`or` (variadic arity ≠ 2) keep their fold; any other shape → `none`. Via `if op == …`
  -- (NOT a `match op with "and"/"or"` string-literal matcher, which — like `match_10` — would compile to a
  -- non-reducible sparse matcher and re-block `split`/`simp`); byte-identical to the former string match.
  else if op == "and" then
    (if (consts.filterMap id).all (· == .bool true) then some (.bool true)
     else if (consts.filterMap id).any (· == .bool false) then some (.bool false) else none)
  else if op == "or" then
    (if (consts.filterMap id).any (· == .bool true) then some (.bool true)
     else if (consts.filterMap id).all (· == .bool false) then some (.bool false) else none)
  else none

/-- Conservatively: could EVALUATING this expression trap (divide-by-zero / overflow / shift-out-of-range)?
True if it contains ANY arithmetic or bitwise application anywhere (`+ - * / %`, shifts) — those can trap;
comparisons/booleans do not themselves trap but their operands might, so recurse; `var`/`const` never trap.
Used to GUARD the equal-branch `if` collapse: dropping a condition is sound ONLY if the condition cannot
trap (else `(if <trapping-c> a a)` — which traps — would wrongly collapse to `a`, a FALSE "proven"). -/
def mayTrap : SymExpr → Bool
  | .var _ => false
  | .const _ => false
  | .app op args => arithOps.contains op || bitwiseOps.contains op || args.attach.any (fun x => mayTrap x.val)
  | .ite c t e => mayTrap c || mayTrap t || mayTrap e
  | .tuple es => es.attach.any (fun x => mayTrap x.val)
  | .record fs => fs.attach.any (fun x => mayTrap x.val.2)
  | .ctor _ args => args.attach.any (fun x => mayTrap x.val)
  | .proj b _ => mayTrap b
  | .case s arms => mayTrap s || arms.attach.any (fun x => mayTrap x.val.2)
  | .localFn _ _ _ => false  -- binding a local fn is a pure value construction; it never traps
termination_by e => sizeOf e
decreasing_by
  all_goals simp_wf
  all_goals
    first
      | omega
      | (have h := Array.sizeOf_lt_of_mem x.property; omega)
      | (rcases x with ⟨⟨k, e⟩, hmem⟩; have h := Array.sizeOf_lt_of_mem hmem; simp_all; omega)

/-- Is `e` the literal `.const (.int n)`? A REDUCIBLE STRUCTURAL check (a `match` + `Int` `==`, which is
NOT opaque), unlike `e == .const (.int n)` whose `SymExpr` derived `BEq` is `opaque` (kernel-irreducible).
Behavior-identical to `e == .const (.int n)` for every `e` (both true iff `e` is exactly that const int).
This lets the `normalizeAppIdentities` const-IDENTITY guards REDUCE in proofs and gives the syntactic
`e = .const (.int n)` fact (`isConstInt_eq`) the capstone `.app`-identity soundness needs — which the opaque
`==` could not supply. -/
def isConstInt (e : SymExpr) (n : Int) : Bool := match e with | .const (.int m) => m == n | _ => false
/-- Bool companion of `isConstInt` (reducible; for the `and`/`or` identity guards). -/
def isConstBool (e : SymExpr) (b : Bool) : Bool := match e with | .const (.bool c) => c == b | _ => false

/-- Canonicalize a symbolic expression by SOUND rewrites only: recurse into subterms; SOUND constant
folding of comparison/boolean ops (`foldConst?`); an `if` on a (now possibly-folded) constant boolean
selects its branch; an `if` whose branches are identical collapses. Deliberately does NOT fold or
reassociate ARITHMETIC (needs width/overflow-trap-aware semantics — T2.0d; an unsound fold = a FALSE
"proven"). Folding a comparison/boolean CONDITION composes with `if`-selection to prove more of an
optimizer's branch-elimination rewrites. -/
-- 🛡️ SOUNDNESS RULE FOR ADDING AN IDENTITY HERE (distilled from two false-`proven` bugs the Lean
-- capstone audit caught — collapse #6533, `x-x→0` #6541): a rewrite MUST be meaning-preserving
-- (`denote(rewrite) = denote(orig)`) for EVERY input INCLUDING floats and traps. `normalize` is
-- TYPE-ERASED (a `var`/subexpr may be int OR float). The trap is an identity that fires on `a == b`
-- (or produces `.int 0`) for an operator that is ALSO valid on FLOATS and has NO int-literal operand
-- forcing an int context: e.g. `x - x → 0` is WRONG for a float `x` (`NaN-NaN=NaN`; `.f64 0.0 ≠ .int 0`),
-- and `SymExpr`'s derived `==` uses IEEE float equality (`+0.0 == -0.0` is `true`) so it must NOT gate a
-- structural-identity collapse over float branches. SAFE patterns: an `int`-literal operand forces int
-- typing (`x+0`,`x*0`,`x/1`,`x%1` — `float ⊕ int` is ill-typed, never compiles); integer-only ops
-- (`&`,`|`,`^`,`<<`,`>>` — bitwise on floats is ill-typed); bool-only ops (`and`/`or`/`not`); comparing a
-- concrete value with `Value.valueEqSpec` (bit-faithful) instead of the derived `==`.
-- REFACTORED (mirrors the `foldConst?` #6892/#7005 arc): a `not` size-1 dispatch + a `size == 2` binary
-- SIZE-DISPATCH of `if op == …` chains, NOT a `match op, #[a,b]` sparse matcher. The `match op, args' with
-- | "+", #[a,b] => …` form compiled to a non-reducible matcher (like `foldConst?.match_10`) that neither
-- `split` nor `simp` can case — blocking the capstone `denote (normalize e) = denote e`'s `.app`-identity
-- soundness (which must case this def). Built only from `if op == …`/`Value` matches, it now REDUCES in
-- proofs (`by_cases hop : (op == "…") = true; rw [if_pos/if_neg]`, the #6997 tactic). BYTE-IDENTICAL to the
-- former arms — verified by the full `#guard` identity suite below + the nix `oracle-lean-smoke`.
def normalizeAppIdentities (op : String) (args' : Array SymExpr) : SymExpr :=
  let isI := isConstInt
  let isB := isConstBool
  -- DOUBLE-NEGATION `not (not x) → x`: the two `not`s evaluate `x` once and cancel (bool involution),
  -- with the same trap behavior as `x` — operand-preserving, no guard (matches the identity discipline
  -- below: sound for well-typed bool `x`; an ill-typed `x` never compiles into the differential).
  if op == "not" && args'.size == 1 then
    -- `not (not x) → x`. Cased via constructor + `oa.size == 1` dispatch (NOT a `some (.app "not" #[inner])`
    -- array-literal pattern, which — like the former op-matcher — compiles to a non-reducible sparse matcher
    -- `split`/`by_cases` cannot case). Byte-identical: fires iff the sole arg is `.app "not"` of one element.
    (match args'[0]! with
     | SymExpr.app o oa => if o == "not" && oa.size == 1 then oa[0]! else .app op args'
     | _ => .app op args')
  else if args'.size == 2 then
    let a := args'[0]!
    let b := args'[1]!
    if op == "+" then (if isI b 0 then a else if isI a 0 then b else .app op args')
    -- `x-0→x` PRESERVES the operand (the `int 0` literal forces an int context; `- float int` is ill-typed).
    -- SOUNDNESS: NO `x-x→0` here. `-` is valid on FLOATS too, and `normalize` is type-erased (a `var` may be
    -- float), so `(- x x)` on a float `x` would wrongly fold to `.int 0` — but `x - x` is `.f64 (x-x)`
    -- (NaN for x=NaN/inf; `.f64 0.0 ≠ .int 0` even when finite). It is NOT meaning-preserving, so it is
    -- removed (was an unsound completeness fold). (`x^x→0` is safe: `^` is integer-only, so `^` on floats is
    -- ill-typed and never compiles.)
    else if op == "-" then (if isI b 0 then a else .app op args')
    else if op == "*" then
      (if isI b 1 then a else if isI a 1 then b
       else if isI b 0 && !mayTrap a then SymExpr.const (Value.int 0)
       else if isI a 0 && !mayTrap b then SymExpr.const (Value.int 0)
       else .app op args')
    -- DIVISION/MODULO by the literal 1 — divisor 1 is never 0 and never the INT_MIN/-1 overflow case, so
    -- these never trap. `x/1=x` PRESERVES the operand (no guard); `x%1=0` for all x DROPS the dividend →
    -- `!mayTrap` guard. Only the literal-1 divisor is folded (general `/`,`%` stay deferred: trap-conditional).
    else if op == "/" then (if isI b 1 then a else .app op args')
    else if op == "%" then (if isI b 1 && !mayTrap a then SymExpr.const (Value.int 0) else .app op args')
    else if op == "or" then
      (if isB a true then SymExpr.const (Value.bool true)
       else if isB a false then b
       else if isB b false then a
       else if isB b true && !mayTrap a then SymExpr.const (Value.bool true)
       -- IDEMPOTENCE `x or x → x` (bool companion of `x|x→x`): PRESERVES the operand (both sides evaluate x,
       -- with the same trap), so no `!mayTrap` guard — sound like `x&x→x`/`x|x→x`.
       else if symExprEqB a b then a
       else .app op args')
    else if op == "and" then
      (if isB a false then SymExpr.const (Value.bool false)
       else if isB a true then b
       else if isB b true then a
       else if isB b false && !mayTrap a then SymExpr.const (Value.bool false)
       else if symExprEqB a b then a  -- `x and x → x` idempotence (operand-preserving, like `x or x → x`)
       else .app op args')
    -- SOUND BITWISE identities — WIDTH-INDEPENDENT (0 is all-zero bits, `<<`/`>>` by 0 is identity at any
    -- width), the bit-op companions of the integer ones above. `x&0`/`0&x`→0 DROPS the operand → `!mayTrap`
    -- guard; `x|0`/`x^0`/`x<<0`/`x>>0`→x and `x&x`/`x|x`→x PRESERVE the operand (incl. its traps).
    else if op == "&" then
      (if isI b 0 && !mayTrap a then SymExpr.const (Value.int 0)
       else if isI a 0 && !mayTrap b then SymExpr.const (Value.int 0)
       else if symExprEqB a b then a
       else .app op args')
    else if op == "|" then
      (if isI b 0 then a else if isI a 0 then b else if symExprEqB a b then a else .app op args')
    -- `x^0`/`0^x`→x PRESERVE the operand; `x^x`→0 (XOR of equal operands is all-zero at ANY width, the
    -- common zeroing idiom; the XOR companion of `x-x→0`/`x&0→0`) DROPS the operand → `!mayTrap` guard.
    else if op == "^" then
      (if isI b 0 then a else if isI a 0 then b
       else if symExprEqB a b && !mayTrap a then SymExpr.const (Value.int 0)
       else .app op args')
    else if op == "<<" then (if isI b 0 then a else .app op args')
    else if op == ">>" then (if isI b 0 then a else .app op args')
    else .app op args'
  else .app op args'

/-- Does the expression contain NO float (`.f64`) constant anywhere? The equal-branch `ite` collapse
(`if c then t else e` with `t == e` → `t`) compares branches with `SymExpr`'s derived `BEq`, whose float
leaves use IEEE `==` — and `+0.0 == -0.0` is `true` though the two are OBSERVABLY distinct
(`1.0 / +0.0 = +inf` vs `-inf`). So collapsing float-containing branches is NOT meaning-preserving (a
false `proven`). This gate restricts the collapse to float-free branches, where the derived `BEq` is a
faithful structural equality. (A bit-faithful float comparison would recover the float case; deferred.) -/
def symFloatFree : SymExpr → Bool
  | .const (.f64 _) => false
  | .const _ => true
  | .var _ => true
  | .app _ args => args.attach.all (fun x => symFloatFree x.val)
  | .ite c t e => symFloatFree c && symFloatFree t && symFloatFree e
  | .tuple es => es.attach.all (fun x => symFloatFree x.val)
  | .record fs => fs.attach.all (fun x => symFloatFree x.val.2)
  | .ctor _ args => args.attach.all (fun x => symFloatFree x.val)
  | .proj b _ => symFloatFree b
  | .case s arms => symFloatFree s && arms.attach.all (fun x => symFloatFree x.val.2)
  | .localFn _ _ _ => true  -- a local-fn binding carries no float leaf of its own
termination_by e => sizeOf e
decreasing_by
  all_goals simp_wf
  all_goals first
    | omega
    | (have h := Array.sizeOf_lt_of_mem x.property; omega)
    | (rcases x with ⟨⟨k, e⟩, hmem⟩; have h := Array.sizeOf_lt_of_mem hmem; simp_all; omega)

/-- Canonicalize a symbolic expression by SOUND rewrites; `normalizeAppIdentities` carries the
`app`-arm algebraic identities (split out so this matcher stays simple enough for equation-lemma
generation). -/
def normalize : SymExpr → SymExpr
  | .var n => .var n
  -- canonicalize a float constant to its `.f64` value so a float LITERAL (`.float`) and a computed/folded
  -- float (`.f64`) with the same value compare structurally equal (else a folded `(+ 475.0 514.0)`=.f64
  -- would not match the literal `989.0`=.float). A non-float const (int/bool/…) is unchanged.
  | .const v => .const (match Value.asF64? v with | some f => .f64 f | none => v)
  | .app op args =>
    let args' := args.attach.map (fun x => normalize x.val)
    match foldConst? op args' with
    | some v => .const v
    | none => normalizeAppIdentities op args'
  | .tuple es => .tuple (es.attach.map (fun x => normalize x.val))
  | .record fs => .record (fs.attach.map (fun x => (x.val.1, normalize x.val.2)))
  | .ctor tag args => .ctor tag (args.attach.map (fun x => normalize x.val))
  | .proj b s => .proj (normalize b) s
  | .case s arms => .case (normalize s) (arms.attach.map (fun x => (x.val.1, normalize x.val.2)))
  | .localFn s b c => .localFn s b c  -- inert under normalization (inlined at call sites; cap kept as-is)
  | .ite c t e =>
    match normalize c with
    | .const (.bool true) => normalize t
    | .const (.bool false) => normalize e
    | c' =>
      let t' := normalize t
      let e' := normalize e
      -- BOOLEAN MATERIALIZATION: `if c then true else false → c`, `if c then false else true → not c`.
      -- OPERAND-PRESERVING (c' is evaluated once with the SAME trap on both sides — the ite forces c
      -- first), so NO `!mayTrap` guard; sound for a well-typed bool `c'` (an ill-typed c never compiles).
      -- A common backend idiom; folding it here shrinks normalized-but-different (a bool-returning `if`).
      -- STRUCTURAL match (not `t' == .const …`): `SymExpr`'s derived `BEq` recurses over `Array SymExpr`
      -- and is NOT kernel-reducible, so a `==`-against-a-literal defeats the soundness proofs (`decide`/
      -- `simp`/`split` can't reduce it); a `match` on the ctor IS reducible. Behavior-identical.
      match t', e' with
      | .const (.bool true), .const (.bool false) => c'
      | .const (.bool false), .const (.bool true) => .app "not" #[c']
      -- collapse identical branches ONLY when the condition can't trap (dropping a trapping condition
      -- would unsoundly claim `(if <trapping-c> a a)` — which traps — equal to `a`) AND the branch is
      -- FLOAT-FREE (`symFloatFree`): the derived `==` uses IEEE float equality where `+0.0 == -0.0`, so
      -- collapsing float branches is not meaning-preserving (`1.0/+0.0` ≠ `1.0/-0.0`) — a false `proven`.
      | _, _ => if symExprEqB t' e' && !mayTrap c' && symFloatFree t' then t' else .ite c' t' e'
termination_by e => sizeOf e
decreasing_by
  all_goals simp_wf
  all_goals
    first
      | omega
      | (have h := Array.sizeOf_lt_of_mem x.property; omega)
      | (rcases x with ⟨⟨k, e⟩, hmem⟩; have h := Array.sizeOf_lt_of_mem hmem; simp_all; omega)

/-- A symbolic environment: each program parameter name bound to its symbolic variable. -/
abbrev SymEnv := List (ByteArray × SymExpr)

/-- Match a pattern against a CONCRETE symbolic value. Result: `some (some env)` = the pattern matches,
binding `env`; `some none` = it definitely does NOT match (try the next arm); `none` = CANNOT DECIDE (the
scrutinee is symbolic — a `var`/`app`/`ite`, not a concrete `ctor`/`tuple`/`const`), so the caller must
`cannotProve` the whole match. Handles: `_` wildcard, a bare-name binder, `None`/`(Some|Ok|Err p)` ctor
patterns, `(tuple p…)`, and integer-literal patterns. Any other pattern (guard, qualified ctor, record
pattern) → `none` (undecidable, boundary). SOUND: only decides against a concrete value. -/
partial def symMatchPat (m : Module) (patId : Nat) (v : SymExpr) : Option (Option SymEnv) :=
  match m.nodes[patId]? with
  | some (Node.atom lid) =>
    match m.leaves[lid]? with
    | some (Leaf.name b) =>
      if b == "_".toUTF8 then some (some [])
      else if b == "None".toUTF8 then
        match v with
        | .ctor t _ => if t == "None".toUTF8 then some (some []) else some none
        | .const _ => some none
        | .tuple _ => some none
        | .record _ => some none
        | _ => none
      else some (some [(b, v)])
    | some l =>
      (match Value.ofLeaf l with
       | some lit => (match v with
                      | .const cv => if Value.valueEqSpec cv lit then some (some []) else some none
                      | .ctor _ _ => some none
                      | .tuple _ => some none
                      | .record _ => some none
                      | _ => none)
       | none => none)
    | none => none
  | some (Node.list pc) =>
    match m.headName? (Node.list pc) with
    | some h =>
      if h == "None".toUTF8 then
        -- a NULLARY `(None)` ctor pattern (the compiler emits nullary ctor patterns as 0-arg application
        -- lists, not bare atoms) — matches a `.ctor "None"` scrutinee with NO bindings. The list-form
        -- companion of the bare-atom `None` case above; without it `(match <None> … (None d))` was
        -- "match arm undecidable" (None is a built-in, not a declared ctor, so `ctorAppName?` declines).
        -- (v-cdz-smith: the ABSENT-key Map.lookup → concrete None cases.)
        match v with
        | .ctor t _ => if t == "None".toUTF8 then some (some []) else some none
        | .const _ => some none
        | .tuple _ => some none
        | .record _ => some none
        | _ => none
      else if h == "Some".toUTF8 || h == "Ok".toUTF8 || h == "Err".toUTF8 then
        match v with
        | .ctor t args => if t == h then (match pc[1]?, args[0]? with
                                          | some sp, some payload => symMatchPat m sp payload
                                          | _, _ => some none)
                          else some none
        | .const _ => some none
        | .tuple _ => some none
        | .record _ => some none
        | _ => none
      else if h == "tuple".toUTF8 then
        match v with
        | .tuple es =>
          let sps := pc.extract 1 pc.size
          if sps.size != es.size then some none
          else (sps.zip es).foldl (fun (acc : Option (Option SymEnv)) p =>
            match acc with
            | some (some env) => (match symMatchPat m p.1 p.2 with
                                  | some (some e2) => some (some (e2 ++ env))
                                  | some none => some none
                                  | none => none)
            | other => other) (some (some []))
        | .const _ => some none
        | .ctor _ _ => some none
        | .record _ => some none
        | _ => none
      else if h == "record".toUTF8 then
        -- `(record (k p)…)` matches a record scrutinee BY FIELD (each named key must be present; a partial
        -- pattern checks only its own fields) — mirrors Eval.matchRecordPats.
        (match v with
         | .record vfields =>
           (pc.extract 1 pc.size).foldl (fun (acc : Option (Option SymEnv)) fp =>
             match acc with
             | some (some env) =>
               (match recordField? m fp with
                | some (key, subPatId) =>
                  (match (vfields.find? (fun kv => kv.1 == key)).map (·.2) with
                   | some fv => (match symMatchPat m subPatId fv with
                                 | some (some e2) => some (some (env ++ e2))
                                 | some none => some none
                                 | none => none)
                   | none => some none)
                | none => none)
             | other => other) (some (some []))
         | .const _ => some none | .ctor _ _ => some none | .tuple _ => some none | _ => none)
      -- a USER (or prelude) sum-constructor pattern `(C p…)` / `((. T C) p…)`, mirroring symCtorConstruct's
      -- erasure so pattern and value use ONE representation: a NEWTYPE pattern binds the erased payload
      -- directly; a struct-newtype matches the field tuple; a sole-nullary matches `unit`; a tagged ctor
      -- matches `.ctor cname` (arity 1 → payload; arity ≥2 → the tuple payload). Wrong tag → no-match.
      else match ctorAppName? m pc with
        | none => none
        | some cname =>
          if newtypeCtor? m cname then (match pc[1]? with | some sp => symMatchPat m sp v | none => some (some []))
          else if structNewtypeCtor? m cname then
            (match v with
             | .tuple es =>
               let sps := pc.extract 1 pc.size
               if sps.size != es.size then some none
               else (sps.zip es).foldl (fun (acc : Option (Option SymEnv)) p =>
                 match acc with
                 | some (some env) => (match symMatchPat m p.1 p.2 with | some (some e2) => some (some (e2 ++ env)) | some none => some none | none => none)
                 | other => other) (some (some []))
             | .const _ => some none | .ctor _ _ => some none | .record _ => some none | _ => none)
          else if soleNullaryCtor? m cname then
            (match v with
             | .const cv => if cv == Value.unit then some (some []) else some none
             | .ctor _ _ => some none | .tuple _ => some none | .record _ => some none | _ => none)
          else match variantCtorArity? m cname with
            | none => none
            | some ar =>
              (match v with
               | .ctor t args =>
                 if t != cname then some none
                 else if ar == 0 then some (some [])
                 else if ar == 1 then (match pc[1]?, args[0]? with
                                       | some sp, some p => symMatchPat m sp p
                                       | some _, none => some none
                                       | none, _ => some (some []))
                 else (match args[0]? with
                       | some (SymExpr.tuple es) =>
                         let sps := pc.extract 1 pc.size
                         if sps.size != es.size then some none
                         else (sps.zip es).foldl (fun (acc : Option (Option SymEnv)) p =>
                           match acc with
                           | some (some env) => (match symMatchPat m p.1 p.2 with | some (some e2) => some (some (e2 ++ env)) | some none => some none | none => none)
                           | other => other) (some (some []))
                       | _ => some none)
               | .const _ => some none | .tuple _ => some none | .record _ => some none | _ => none)
    | none => none
  | none => none

/-- The discriminant TAG of a match pattern, for structurally identifying an arm of a symbolic `case`
(a ctor name, `None`, `tuple`, or `_` for a wildcard/binder). `none` = a pattern the symbolic case cannot
model (a literal, or a nested/complex form) → the whole match is undecidable. -/
partial def symArmTag? (m : Module) (patId : Nat) : Option ByteArray :=
  match m.nodes[patId]? with
  | some (Node.atom lid) =>
    (match m.leaves[lid]? with
     | some (Leaf.name b) =>
       if b == "None".toUTF8 then some "None".toUTF8
       else if (variantCtorArity? m b).isSome then some b
       else some "_".toUTF8
     | _ => none)
  | some (Node.list pc) =>
    (match m.headName? (Node.list pc) with
     | some h => if h == "tuple".toUTF8 || h == "record".toUTF8 || h == "Some".toUTF8 || h == "Ok".toUTF8 || h == "Err".toUTF8 then some h
                 else ctorAppName? m pc
     | none => none)
  | none => none

/-- Bind a match pattern's variables to symbolic PROJECTIONS of the (symbolic) scrutinee, for a `case` arm.
`none` = a pattern the symbolic case cannot model (arity ≥2 / newtype / struct / literal / nested-complex).
Mirrors symMatchPat's parsing, but since the scrutinee is symbolic it produces `proj`-bound variables. -/
partial def symBindPat (m : Module) (patId : Nat) (scrut : SymExpr) : Option (List (ByteArray × SymExpr)) :=
  match m.nodes[patId]? with
  | some (Node.atom lid) =>
    (match m.leaves[lid]? with
     | some (Leaf.name b) =>
       if b == "_".toUTF8 then some []
       else if b == "None".toUTF8 then some []
       else if (variantCtorArity? m b).isSome then some []
       else some [(b, scrut)]
     | _ => none)
  | some (Node.list pc) =>
    (match m.headName? (Node.list pc) with
     | some h =>
       if h == "Some".toUTF8 || h == "Ok".toUTF8 || h == "Err".toUTF8 then
         (match pc[1]? with | some sp => symBindPat m sp (.proj scrut h) | none => some [])
       else if h == "tuple".toUTF8 then
         (pc.extract 1 pc.size).toList.zip (List.range (pc.size - 1)) |>.foldl (fun acc p =>
           match acc with
           | none => none
           | some bs => (match symBindPat m p.1 (.proj scrut (toString p.2).toUTF8) with
                         | some b2 => some (bs ++ b2) | none => none)) (some [])
       else if h == "record".toUTF8 then
         -- `(record (k p)…)` binds each field sub-pattern p to `proj scrut k` (the field selector).
         (pc.extract 1 pc.size).foldl (fun (acc : Option (List (ByteArray × SymExpr))) fp =>
           match acc with
           | none => none
           | some bs => (match recordField? m fp with
                         | some (key, subPatId) => (match symBindPat m subPatId (.proj scrut key) with
                                                    | some b2 => some (bs ++ b2) | none => none)
                         | none => none)) (some [])
       else match ctorAppName? m pc with
         | some cname => (match variantCtorArity? m cname with
                          | some 1 => (match pc[1]? with | some sp => symBindPat m sp (.proj scrut cname) | none => some [])
                          | some 0 => some []
                          | _ => none)
         | none => none
     | none => none)
  | none => none

/-- Is this symbolic value a CONCRETE constructor/tuple/record/const (a match can be DECIDED), vs a symbolic
form (var/app/ite/proj/case) that requires a symbolic `case`? -/
def isConcreteSym : SymExpr → Bool
  | .const _ => true | .ctor _ _ => true | .tuple _ => true | .record _ => true | _ => false

/-- Call-inlining depth bound. A non-recursive call chain inlines within this; a recursive function
exhausts it → `cannotProve` (sound — proving a recursive function's equivalence needs induction, which the
symbolic evaluator does not do). 64 comfortably covers realistic non-recursive helper nesting. -/
def symDefaultFuel : Nat := 64

/-- The body of a TOP-LEVEL VALUE def `(def name VALUE)` whose target is a BARE ATOM named `name` (exactly
3 children: `def`, target-atom, body). A LIST target `(def (f …) …)` is a FUNCTION (even nullary → a
closure value), NOT a value, so it is EXCLUDED — a bare ref to a function is a higher-order value we don't
model. `some bodyId` if found. Used to resolve a bare reference to a top-level constant: `symEval` runs only
`main`'s body, so top-level value defs are otherwise `free name` (evalNode binds them via the root do-env). -/
def topLevelValueDefBody? (m : Module) (name : ByteArray) : Option Nat :=
  match m.nodes[m.root]? with
  | some (Node.list stmts) =>
    stmts.toList.findSome? (fun sid =>
      match asDef? m sid with
      | some dc =>
        if dc.size == 3 then
          match dc[1]? with
          | some targetId =>
            (match m.nodes[targetId]? with
             | some (Node.atom lid) =>
               (match m.leaves[lid]? with
                | some (Leaf.name b) => if b == name then dc[2]? else none
                | _ => none)
             | _ => none)  -- list target = a function def, not a value
          | none => none
        else none
      | none => none)
  | _ => none

mutual
/-- Symbolically evaluate the node `i` under `senv` (params → symbolic vars). Covers the ANALYZABLE SCALAR
FRAGMENT: a bound parameter → its var; a scalar literal → `const`; `(if c t e)` → `ite`; a `(: e T)`
ascription → its value (type carried structurally — both programs ascribe the same, and Rational grounding
etc. is a future increment); an arithmetic/comparison/boolean operator → `app`; a `(let ((n v)…) body)`
→ sequentially bind each `n` to `symEval v` (let*, later bindings see earlier) then `symEval body`
(substitution matches Cadenza's lazy let — an unused binding vanishes; SOUND-conservative: if any binding
value is unmodelable it sinks the whole `let`, since an eager/discarded binding's trap can't be ruled out).
Everything else — match/sum, collections, calls, recursion — is the incompleteness boundary → `cannotProve`
(honest; degrade to the sampled differential there). Sound: never invents a value for an unmodeled construct. -/
partial def symEval (m : Module) (senv : SymEnv) (fuel : Nat) (ty : IntTy) (i : Nat) : SymOutcome :=
  match m.nodes[i]? with
  | some (Node.atom lid) =>
    match m.leaves[lid]? with
    | some (Leaf.name b) =>
      match senv.find? (fun p => p.1 == b) with
      | some (_, e) => .sym e
      | none =>
        if b == "None".toUTF8 then .sym (.ctor "None".toUTF8 #[])
        else match topLevelValueDefBody? m b with
          -- a bare reference to a TOP-LEVEL VALUE def `(def b VALUE)` → its body's value (top-level defs are
          -- in scope; evalNode binds them via the root do-env, so this matches eval). Fuel bounds def→def
          -- reference chains. A function-shaped def / non-def name stays a free name (cannotProve).
          | some bodyId => if fuel == 0 then .cannotProve "symeval: free-name value-def fuel exhausted"
                           else symEval m [] (fuel - 1) defaultIntTy bodyId
          | none =>
            -- a bare reference to a NULLARY user CONSTRUCTOR declared in a top-level `(type …)` (e.g. `Blue`
            -- from `(type Color (Red)(Green)(Blue))`) → its ctor value. Mirrors `symCtorConstruct`'s erasure
            -- for a 0-arg application: a SOLE-nullary ctor erases to `unit`; any other nullary ctor → the
            -- tagged `.ctor name #[]`. (Newtype/struct-newtype are arity ≥1, so never a bare nullary ref.)
            if soleNullaryCtor? m b then .sym (.const .unit)
            else match variantCtorArity? m b with
              | some 0 => .sym (.ctor b #[])
              | _ => .cannotProve "symeval: free name (not a bound parameter)"
    | some l =>
      match Value.ofLeaf l with
      | some v => .sym (.const v)
      | none => match l with
                -- a SYMBOL leaf (`#"…"`, e.g. a Qty base-unit name `(Unit.base #"second")`) → its raw bytes.
                -- Symbolic-only (eval leaves symbols `.unsupported`; the cadenza-equiv differential is
                -- symEval-vs-roundtrip, so decoding a symbol to its bytes is CONSISTENT across P/P' — symbols
                -- compare by bytes, and stay DISTINCT from `.str` (different `Value` ctor) so no false
                -- symbol==string. Unblocks Qty programs — the unit name is a symbol, so the operand atom hit
                -- this leaf path before `Unit.base` could wrap it (v-cdz-smith #7290 migration to non-scalar-leaf).
                | .sym b => .sym (.const (.bytes b))
                | _ => .cannotProve "symeval: non-scalar leaf"
    | none => .cannotProve "symeval: leaf index out of range"
  | some (Node.list children) =>
    -- CONSTRUCTION first: a head resolving to a declared sum constructor (bare `C` or qualified `(. T C)`).
    match symCtorConstruct m senv fuel ty children with
    | some o => o
    | none =>
    match m.headName? (Node.list children) with
    | some h =>
      if h == "if".toUTF8 then
        match children[1]?, children[2]?, children[3]? with
        | some cId, some tId, some eId =>
          match symEval m senv fuel ty cId, symEval m senv fuel ty tId, symEval m senv fuel ty eId with
          | .sym c, .sym t, .sym e => .sym (.ite c t e)
          | .cannotProve r, _, _ => .cannotProve r
          | _, .cannotProve r, _ => .cannotProve r
          | _, _, .cannotProve r => .cannotProve r
        | _, _, _ => .cannotProve "symeval: malformed if"
      else if h == ":".toUTF8 then
        -- `(: e T)`: evaluate `e` at the ASCRIBED integer width (so arithmetic inside folds/overflows at the
        -- right width) — a non-integer `T` (Float/Rational/…) leaves the ambient `ty` unchanged.
        match children[1]?, children[2]? with
        | some vId, some tyId => symEval m senv fuel ((parseIntTy? m tyId).getD ty) vId
        | some vId, none => symEval m senv fuel ty vId
        | _, _ => .cannotProve "symeval: malformed ascription"
      else if h == "let".toUTF8 then
        match children[1]?, children[2]? with
        | some bindingsId, some bodyId =>
          match m.nodes[bindingsId]? with
          | some (Node.list pairs) => symLet m senv fuel ty pairs.toList bodyId
          | _ => .cannotProve "symeval: let bindings not a list"
        | _, _ => .cannotProve "symeval: malformed let"
      else if h == "do".toUTF8 then
        -- an inline `(do stmt… last)` expression → sequential def-bindings + discarded non-defs + last.
        symDo m senv fuel ty children
      else if h == "fn".toUTF8 then
        -- a LAMBDA `(fn (param…) body)` → a `.localFn` symbolic CLOSURE over the def-site `senv`, exactly like
        -- the `do`-def local-fn binding (below) — so a later application (a param bound to it, then applied)
        -- INLINES it via the local-fn call path. Byte-faithful to `evalFn` (Eval.lean:1554): its params are
        -- the WHOLE param-list node's children (NOT `paramSpecNodes`, which drops the leading name of a NAMED
        -- def target — a lambda's param list `(y)` has no leading name to drop), body = `children[2]`, capture
        -- = the current env. Closes v-cdz-smith's HIGHER-ORDER cluster (a `(fn …)` passed to a local fn and
        -- applied): the arg previously symEval'd to `cannotProve` ("argument unmodelable"); now it is a closure.
        (match children[1]?, children[2]? with
         | some paramListId, some bodyId =>
           let specs := match m.nodes[paramListId]? with | some (Node.list ps) => ps | _ => #[]
           .sym (.localFn specs bodyId senv)
         | _, _ => .cannotProve "symeval: malformed fn")
      else if h == "try".toUTF8 then
        -- `(try e)` — the `?` operator. Model only the SUCCESS unwrap: a concrete `Ok v` / `Some v` → `v`
        -- (byte-faithful to evalTry, Eval.lean:1487-1494). The FAILURE cases (`Err`/`None`) short-circuit to
        -- the function boundary via `errReturn`, which symEval does not thread → cannotProve (conservative).
        -- A symbolic operand (can't tell success from failure) → cannotProve.
        (match children[1]? with
         | some eId =>
           (match symEval m senv fuel ty eId with
            | .sym (.ctor t #[v]) =>
              if t == "Ok".toUTF8 || t == "Some".toUTF8 then .sym v
              else .cannotProve "symeval: try on a failing ctor (errReturn short-circuit not modeled)"
            | .cannotProve r => .cannotProve r
            | _ => .cannotProve "symeval: try operand not a concrete Ok/Some (short-circuit/symbolic not modeled)")
         | none => .cannotProve "symeval: malformed try")
      else if h == "tuple".toUTF8 then
        -- a tuple value (lazy elements). Build `.tuple` of the element SymExprs; an unmodelable element
        -- sinks the whole tuple (conservative — the value can't be fully compared).
        let outs := (children.extract 1 children.size).map (fun c => symEval m senv fuel ty c)
        match outs.findSome? (fun o => match o with | .cannotProve r => some r | .sym _ => none) with
        | some r => .cannotProve r
        | none => .sym (.tuple (outs.map (fun o => match o with | .sym e => e | .cannotProve _ => .const .unit)))
      else if h == "list".toUTF8 then
        -- a LIST literal (ordered; STRICT in its elements, #5194). Model as `.ctor "list"` of the element
        -- SymExprs — DISTINCT from `.tuple`, and ORDERED so structural equality is faithful (two list
        -- literals are equal iff same elements in order). Coverage: list literals now get a verdict instead
        -- of falling to `cannotProve` (the head was previously unmodeled → treated as a call → cannotProve).
        let outs := (children.extract 1 children.size).map (fun c => symEval m senv fuel ty c)
        match outs.findSome? (fun o => match o with | .cannotProve r => some r | .sym _ => none) with
        | some r => .cannotProve r
        | none => .sym (.ctor "list".toUTF8 (outs.map (fun o => match o with | .sym e => e | .cannotProve _ => .const .unit)))
      else if h == "set".toUTF8 || h == "Set".toUTF8 then
        -- lowercase `(set …)` = a SET LITERAL; CAPITAL `(Set …)` = the value-render form the `--target
        -- cadenza` ROUNDTRIP emits (a const-folded set value renders with the module-name head "Set", NOT a
        -- `#set`/setCtor node) — both are the same canonical set (v-cdz-smith #7412/#7371 roundtrip residual).
        -- a SET literal → `.ctor "set"`. When ALL elements are CONCRETE (incl COMPOUND list/tuple/record via
        -- `symElemToValue?`), CANONICALIZE via eval's own `canonSet` (sort + dedup) then rebuild the SymExpr
        -- form with `valueToSym` — so the symbolic value matches eval's canonicalized set AND set-REORDER
        -- equality proves (`(set 1 2)` ≡ `(set 2 1)` ≡ `(set 1 2 2)`, and now for compound elements too). This
        -- also keeps set LITERALS consistent with `Set.insert` output (both canonical). SYMBOLIC (non-value)
        -- elements → keep SOURCE ORDER (sound; reorder-equality stays incomplete there). An unorderable
        -- all-concrete set (canonSet `none`) is one `evalNode` DECLINES at construction → keep source order
        -- (the differential skips the declined program anyway). Distinct head from `list`/`tuple`.
        let outs := (children.extract 1 children.size).map (fun c => symEval m senv fuel ty c)
        match outs.findSome? (fun o => match o with | .cannotProve r => some r | .sym _ => none) with
        | some r => .cannotProve r
        | none =>
          let elems := outs.map (fun o => match o with | .sym e => e | .cannotProve _ => .const .unit)
          (match elems.mapM symElemToValue? with
           | some vals => (match canonSet vals with
                           | some s => .sym (.ctor "set".toUTF8 (s.map valueToSym))
                           | none => .sym (.ctor "set".toUTF8 elems))
           | none => .sym (.ctor "set".toUTF8 elems))
      else if h == "map".toUTF8 || h == "Map".toUTF8 then
        -- lowercase `(map …)` = a MAP LITERAL; CAPITAL `(Map …)` = the roundtrip value-render form (a
        -- const-folded map value renders with the module-name head "Map") — both the same canonical map.
        -- a MAP literal: each entry is `(k v)` or `(= k v)` (mirrors `evalMapLiteral`'s key/value parse).
        -- Model as `.ctor "map"` of one `.tuple #[key, value]` per entry. When ALL entries are const (key AND
        -- value), CANONICALIZE via eval's own `canonMap` (last-insert-wins per key + sort-by-key) so the
        -- symbolic value matches eval's canonicalized map AND key-REORDER / dup-key equality proves. Symbolic
        -- entries → keep SOURCE ORDER (sound; reorder-equality stays incomplete). Unorderable key (canonMap
        -- `none`, `evalNode` DECLINES) → keep source order (declined program is skipped). Malformed entry sinks.
        let entryOuts := (children.extract 1 children.size).map (fun j =>
          match m.nodes[j]? with
          | some (Node.list ec) =>
            let kv := match m.headName? (Node.list ec) with
              | some hh => if hh == "=".toUTF8 && ec.size == 3 then (ec[1]?, ec[2]?) else (ec[0]?, ec[1]?)
              | none => (ec[0]?, ec[1]?)
            (match kv with
             | (some kId, some vId) =>
               (match symEval m senv fuel ty kId, symEval m senv fuel ty vId with
                | .sym ke, .sym ve => SymOutcome.sym (.tuple #[ke, ve])
                | .cannotProve r, _ => .cannotProve r
                | _, .cannotProve r => .cannotProve r)
             | _ => .cannotProve "symeval: malformed map entry")
          | _ => .cannotProve "symeval: malformed map entry")
        match entryOuts.findSome? (fun o => match o with | .cannotProve r => some r | .sym _ => none) with
        | some r => .cannotProve r
        | none =>
          let entries := entryOuts.map (fun o => match o with | .sym e => e | .cannotProve _ => .const .unit)
          (match entries.mapM (fun e => match e with
                                        | .tuple #[.const k, .const v] => some (k, v)
                                        | _ => none) with
           | some kvs =>
             (match canonMap kvs with
              | some cm => .sym (.ctor "map".toUTF8 (cm.map (fun kv => SymExpr.tuple #[.const kv.1, .const kv.2])))
              | none => .sym (.ctor "map".toUTF8 entries))
           | none => .sym (.ctor "map".toUTF8 entries))
      else if h == "Some".toUTF8 || h == "Ok".toUTF8 || h == "Err".toUTF8 then
        -- a built-in unary Option/Result constructor (lazy payload).
        match children[1]? with
        | some aId => (match symEval m senv fuel ty aId with | .sym e => .sym (.ctor h #[e]) | .cannotProve r => .cannotProve r)
        | none => .cannotProve "symeval: constructor missing payload"
      else if h == "None".toUTF8 then .sym (.ctor "None".toUTF8 #[])
      else if h == "match".toUTF8 then
        -- `(match scrut (pat body)…)`: decidable ONLY when the scrutinee is a CONCRETE ctor/tuple/const.
        -- Try arms in order — a definite match takes its body (bindings extend senv); a definite non-match
        -- falls through; an UNDECIDABLE arm (symbolic scrutinee vs a value-inspecting pattern) → cannotProve.
        match children[1]? with
        | some scrutId =>
          (match symEval m senv fuel ty scrutId with
           | .cannotProve r => .cannotProve r
           | .sym v =>
             if isConcreteSym v then
               -- CONCRETE scrutinee: decide the arm (existing logic).
               let decided := (children.extract 2 children.size).foldl (fun (acc : Option SymOutcome) armId =>
                 match acc with
                 | some o => some o
                 | none =>
                   match (m.nodes[armId]?).bind (fun n => match n with | .list ac => some ac | _ => none) with
                   | some ac => (match ac[0]?, ac[1]? with
                                 | some patId, some bodyId =>
                                   (match symMatchPat m patId v with
                                    | some (some ext) => some (symEval m (ext ++ senv) fuel ty bodyId)
                                    | some none => none
                                    | none => some (.cannotProve "symeval: match arm undecidable (unmodeled pattern)"))
                                 | _, _ => some (.cannotProve "symeval: malformed match arm"))
                   | none => some (.cannotProve "symeval: malformed match arm")) none
               match decided with
               | some o => o
               | none => .cannotProve "symeval: no match arm matched (non-exhaustive?)"
             else
               -- SYMBOLIC scrutinee: build a `.case v arms`, binding each arm's pattern vars to projections
               -- of `v` and symEvaluating the body. An unmodelable pattern/body → the whole match is
               -- undecidable (cannotProve). Arms are kept ORDER-SENSITIVELY so P and P' with the SAME match
               -- (same scrutinee, arms, bodies) prove equal; a reordered/transformed match → cannotProve.
               let armsOpt := (children.extract 2 children.size).foldl (fun (acc : Option (Array (ByteArray × SymExpr))) armId =>
                 match acc with
                 | none => none
                 | some arms =>
                   match (m.nodes[armId]?).bind (fun n => match n with | .list ac => some ac | _ => none) with
                   | some ac => (match ac[0]?, ac[1]? with
                                 | some patId, some bodyId =>
                                   (match symArmTag? m patId, symBindPat m patId v with
                                    | some tag, some binds => (match symEval m (binds ++ senv) fuel ty bodyId with
                                                               | .sym b => some (arms.push (tag, b))
                                                               | .cannotProve _ => none)
                                    | _, _ => none)
                                 | _, _ => none)
                   | none => none) (some #[])
               match armsOpt with
               | some arms => .sym (.case v arms)
               | none => .cannotProve "symeval: match on a symbolic scrutinee has an unmodelable pattern/body")
        | none => .cannotProve "symeval: malformed match (no scrutinee)"
      else if h == "record".toUTF8 then
        -- a record value: (record (f1 v1) (f2 v2)…), fields sorted by key (matching evalRecord's canonical
        -- form + catching a field-reorder). An unmodelable field value sinks the record (conservative).
        let acc := (children.extract 1 children.size).foldl (fun (acc : Option (Array (ByteArray × SymExpr))) j =>
          match acc with
          | none => none
          | some arr =>
            match recordField? m j with
            | some (k, vId) => (match symEval m senv fuel ty vId with | .sym e => some (arr.push (k, e)) | .cannotProve _ => none)
            | none => none) (some #[])
        match acc with
        | some arr => .sym (.record (arr.qsort (fun a b => cmpBytes a.1 b.1 == .lt)))
        | none => .cannotProve "symeval: record has an unmodelable field or a malformed field"
      else if h == ".".toUTF8 && children.size == 3 then
        -- projection `(. base index)`: a NAME index → record field access; an INT index → positional tuple
        -- access; anything else (member/module) → boundary.
        match children[1]?, children[2]? with
        | some tId, some iId =>
          match (m.nodes[iId]?).bind (fun n => match n with | .atom lid => m.leaves[lid]? | _ => none) with
          | some (Leaf.name fld) =>
            -- prelude float CONSTANTS `(. Float64|Float32 nan)` → NaN, `… Infinity` → +∞ (member-ACCESS
            -- values, not record fields). Byte-faithful to evalNode (Eval.lean:2052-2056). (v-cdz-smith
            -- next-tier boundary — negative infinity is `(- Float64.Infinity)`, already handled by `-`.)
            let baseName? := (m.nodes[tId]?).bind (fun n => match n with
                                                            | .atom lid => (match m.leaves[lid]? with | some (Leaf.name b) => some b | _ => none)
                                                            | _ => none)
            let isFloatMod := baseName? == some "Float64".toUTF8 || baseName? == some "Float32".toUTF8
            if isFloatMod && fld == "nan".toUTF8 then .sym (.const .floatNan)
            else if isFloatMod && fld == "Infinity".toUTF8 then .sym (.const (.floatInf false))
            else if baseName? == some "Map".toUTF8 && fld == "empty".toUTF8 then
              -- `(. Map empty)` used as a VALUE = the empty map (prelude module value, Eval.lean:2063-2064).
              -- Mirrors the `((. Map empty))` nullary-call form handled in the member-call dispatch.
              .sym (.ctor "map".toUTF8 #[])
            else
            (match symEval m senv fuel ty tId with
             | .sym (.record fs) => (match fs.find? (fun kv => kv.1 == fld) with
                                     | some (_, e) => .sym e
                                     | none => .cannotProve "symeval: record field not found")
             | .sym _ => .cannotProve "symeval: field projection of a non-record"
             | .cannotProve r => .cannotProve r)
          | some l =>
            (match Value.ofLeaf l with
             | some (.int n) =>
               if n < 0 then .cannotProve "symeval: negative tuple index"
               else (match symEval m senv fuel ty tId with
                     | .sym (.tuple es) => (match es[n.toNat]? with
                                            | some e => .sym e
                                            | none => .cannotProve "symeval: tuple index out of range")
                     | .sym _ => .cannotProve "symeval: positional projection of a non-tuple"
                     | .cannotProve r => .cannotProve r)
             | _ => .cannotProve "symeval: non-positional projection (member not modeled)")
          | none => .cannotProve "symeval: malformed projection index"
        | _, _ => .cannotProve "symeval: malformed projection"
      else match String.fromUTF8? h with
        | some hs =>
          if arithOps.contains hs || bitwiseOps.contains hs || cmpOps.contains hs || hs == "=" || hs == "and" || hs == "or" || hs == "not" then
            let outs := (children.extract 1 children.size).map (fun c => symEval m senv fuel ty c)
            match outs.findSome? (fun o => match o with | .cannotProve r => some r | .sym _ => none) with
            | some r => .cannotProve r
            | none =>
              -- NORMALIZE each operand first: normalize folds width-INDEPENDENT structure (comparisons, `if`
              -- branch-selection, float, let-ground substitutions already done by symEval) to a `.const`, so a
              -- surrounding INTEGER op whose operands only become concrete AFTER those fold (e.g. `(+ (if C a a) b)`
              -- with both branches equal) now sees `.const`s and folds at width `ty` below — closing the
              -- const-fold-to-literal gap the backend exploits (v-cdz-smith FP-0/FP-1). normalize does NOT fold
              -- int arith itself (no width context), so the width-checked fold stays here; idempotent + sound.
              let args := outs.map (fun o => match o with | .sym e => normalize e | .cannotProve _ => .const .unit)
              -- INTEGER const-fold at the ascribed/ambient width `ty`. `evalArithOp` does the width-checked
              -- overflow trap, so fold ONLY when it yields a VALUE (fits): an overflow / div-by-zero /
              -- unsupported keeps the op SYMBOLIC — folding an overflowing case to a value would be a FALSE
              -- 'proven' masking a miscompile (e.g. (+ 200 100) at UInt8 must NOT fold to 300). Comparison /
              -- boolean / float folding stays in `normalize` (width-independent).
              match hs, args with
              | _, #[SymExpr.const (Value.int a), SymExpr.const (Value.int b)] =>
                if arithOps.contains hs then
                  (match evalArithOp hs a b ty with
                   | .value v => .sym (SymExpr.const v)
                   | _ => .sym (SymExpr.app hs args))
                -- BITWISE const-fold at width `ty` via the real `evalBitOp` (same discipline as arith):
                -- fold ONLY when it yields a VALUE; a trapping shift (out-of-range count) / unsupported keeps
                -- the op SYMBOLIC, so a would-trap case is never falsely folded to a value.
                else if bitwiseOps.contains hs then
                  (match evalBitOp hs a b ty with
                   | .value v => .sym (SymExpr.const v)
                   | _ => .sym (SymExpr.app hs args))
                else .sym (SymExpr.app hs args)
              -- RATIONAL const-fold: `(a/b) op (c/d)` via eval's own `rationalArith` (exact, width-INDEPENDENT
              -- — rationals never overflow); fold ONLY when it yields a VALUE (a `/`-by-zero-rational traps →
              -- stays symbolic). Closes the rational-arith FP class (backend const-folds rational arithmetic;
              -- symEval otherwise leaves it `.app`). v-spec-oracle cross-edges #6807 (mul/sub sign) exercise this.
              | _, #[SymExpr.const (Value.rational a b), SymExpr.const (Value.rational c d)] =>
                if arithOps.contains hs then
                  (match rationalArith hs a b c d with
                   | .value v => .sym (SymExpr.const v)
                   | _ => .sym (SymExpr.app hs args))
                else .sym (SymExpr.app hs args)
              -- FLOAT const-fold. Round each op to f32 when the operands mention Float32 — the backend
              -- rounds per-op to f32, so folding at f64 would diverge (v-cdz-smith fp-0); else fold at f64
              -- via evalFloatOp. (Comparison / = / bool over floats stays in normalize's foldConst?.)
              | _, #[SymExpr.const va, SymExpr.const vb] =>
                if arithOps.contains hs then
                  (match Value.asF64? va, Value.asF64? vb with
                   | some x, some y =>
                     (match evalFloatOp hs x y with
                      | .value (.f64 r) =>
                        let isF32 := (children[1]?.map (mentionsFloat32? m)).getD false
                                     || (children[2]?.map (mentionsFloat32? m)).getD false
                        .sym (SymExpr.const (.f64 (if isF32 then (Float.toFloat32 r).toFloat else r)))
                      | _ => .sym (SymExpr.app hs args))
                   | _, _ => .sym (SymExpr.app hs args))
                else .sym (SymExpr.app hs args)
              -- QUANTITY arith over two `Qty` values (`.ctor "qty" #[mag, unit]`): fold the MAGNITUDE (const
              -- ints via `evalArithOp` at width `ty`; else keep the symbolic `.app`) and carry the first unit.
              -- Cadenza's type system guarantees `+`/`-` share a unit; `*` multiplies units, but `Qty.value`
              -- reads only the magnitude so the first unit is faithful for the value-extracting programs
              -- (v-cdz-smith Qty widening #7282). `/`/`%` over Qty stay symbolic (unit division not modeled).
              | _, #[SymExpr.ctor t1 #[m1, u1], SymExpr.ctor t2 #[m2, _]] =>
                if t1 == "qty".toUTF8 && t2 == "qty".toUTF8 && (hs == "+" || hs == "-" || hs == "*") then
                  let mag := match m1, m2 with
                    | SymExpr.const (Value.int a), SymExpr.const (Value.int b) =>
                      (match evalArithOp hs a b ty with | .value v => SymExpr.const v | _ => SymExpr.app hs #[m1, m2])
                    | _, _ => SymExpr.app hs #[m1, m2]
                  .sym (SymExpr.ctor "qty".toUTF8 #[mag, u1])
                else .sym (SymExpr.app hs args)
              | _, _ => .sym (SymExpr.app hs args)
          else
            -- a call `(f arg…)` to a top-level def `f` (not shadowed by a local): INLINE it — bind each
            -- param to its arg's SymExpr (evaluated in the CALLER env), then symEval the callee body in a
            -- FRESH env of just those params (a top-level def sees only its params + globals), fuel-1. Fuel
            -- bounds recursion: a recursive `f` exhausts it → cannotProve (proving a recursive function's
            -- equivalence needs induction, not modeled). A partial application (arity mismatch) → boundary.
            match senv.find? (fun p => p.1 == h) with
            | some (_, .localFn lspecs lbodyId lcap) =>
              -- INLINE a LOCAL FN bound in an enclosing `(do …)`. Same discipline as a top-level call, but the
              -- fresh env is `params ++ lcap` — the CAPTURED def-site env — so the body may reference enclosing
              -- do-bindings (closure), with params SHADOWING same-named captures (params prepended → found
              -- first). Matches eval's eager capture (Eval.lean:1247). Fuel bounds a (self-)recursive local fn.
              if fuel == 0 then .cannotProve "symeval: local-fn call fuel exhausted (recursion?)"
              else
                let nargs := children.size - 1
                if 0 < nargs && nargs < lspecs.size then
                  -- PARTIAL application (CURRYING): fewer args than params. Capture the given args under the
                  -- FIRST params (appended to the existing cap), and return a NEW `.localFn` over the REMAINING
                  -- params — a later application completes it (full-arity → inline below). Byte-faithful to
                  -- `applyClosure`'s partial branch (Eval.lean:1572-1577): `newCap = cap ++ (firstParams.zip args)`,
                  -- `.closure (params.drop nargs) body newCap`. An unmodelable captured arg → cannotProve.
                  let capExt := ((lspecs.extract 0 nargs).zip (children.extract 1 children.size)).foldl
                    (fun (acc : Option (List (ByteArray × SymExpr))) p =>
                      match acc with
                      | none => none
                      | some cap =>
                        match paramSpec? m p.1 with
                        | some (pnm, _) => (match symEval m senv fuel ty p.2 with
                                            | .sym e => some (cap ++ [(pnm, e)])
                                            | .cannotProve _ => none)
                        | none => none) (some lcap)
                  match capExt with
                  | some newCap => .sym (.localFn (lspecs.extract nargs lspecs.size) lbodyId newCap)
                  | none => .cannotProve "symeval: a curried local-fn argument is unmodelable"
                else if lspecs.size != nargs then .cannotProve "symeval: local-fn call arity mismatch (over-application)"
                else
                let callEnv := (lspecs.zip (children.extract 1 children.size)).foldl
                  (fun (acc : Option SymEnv) p =>
                    match acc with
                    | none => none
                    | some env =>
                      match paramSpec? m p.1 with
                      | some (pnm, _) => (match symEval m senv fuel ty p.2 with
                                          | .sym e => some ((pnm, e) :: env)
                                          | .cannotProve _ => none)
                      | none => none) (some lcap)
                match callEnv with
                | some ce => symEval m ce (fuel - 1) defaultIntTy lbodyId
                | none => .cannotProve "symeval: a local-fn call argument is unmodelable"
            | some _ =>
              .cannotProve "symeval: head is a local (non-fn) binding (not a top-level call)"
            | none => match namedParamsBody? m h with
              | some (specs, bodyId) =>
                if fuel == 0 then .cannotProve "symeval: call-inline fuel exhausted (recursion?)"
                else if specs.size != children.size - 1 then
                  .cannotProve "symeval: call arity mismatch (partial application?)"
                else
                  let callEnv := (specs.zip (children.extract 1 children.size)).foldl
                    (fun (acc : Option SymEnv) p =>
                      match acc with
                      | none => none
                      | some env =>
                        match paramSpec? m p.1 with
                        | some (pnm, _) => (match symEval m senv fuel ty p.2 with
                                            | .sym e => some ((pnm, e) :: env)
                                            | .cannotProve _ => none)
                        | none => none) (some ([] : SymEnv))
                  match callEnv with
                  | some ce => symEval m ce (fuel - 1) defaultIntTy bodyId
                  | none => .cannotProve "symeval: a call argument is unmodelable or a param spec is malformed"
              | none => .cannotProve s!"symeval: operator/construct '{hs}' not modeled (boundary)"
        | none => .cannotProve "symeval: non-UTF8 head"
    | none =>
      -- a MEMBER-CALL head `((. Q mem) args)` (non-name head). Model the non-recursive builtin collection
      -- ops here (they are NOT library recursion, so no fuel boundary). First: `List.concat a b` over two
      -- concrete list values → `.ctor "list"` of the appended element arrays (byte-identical to
      -- `List.concat`'s eval). Coverage: list-concat programs get a verdict instead of `cannotProve`.
      match qualHead? m children with
      | some (q, mem) =>
        if q == "List".toUTF8 && mem == "concat".toUTF8 then
          (match children[1]?, children[2]? with
           | some aId, some bId =>
             (match symEval m senv fuel ty aId, symEval m senv fuel ty bId with
              | .sym (.ctor t1 as), .sym (.ctor t2 bs) =>
                if t1 == "list".toUTF8 && t2 == "list".toUTF8 then .sym (.ctor "list".toUTF8 (as ++ bs))
                else .cannotProve "symeval: List.concat on non-list values"
              | .cannotProve r, _ => .cannotProve r
              | _, .cannotProve r => .cannotProve r
              | _, _ => .cannotProve "symeval: List.concat on non-list values")
           | _, _ => .cannotProve "symeval: malformed List.concat")
        else if q == "List".toUTF8 && mem == "len".toUTF8 then
          -- `List.len lst` over a concrete list → the element count (`.ctor "list"` args.size). SOUND: a
          -- LIST keeps duplicates + order, so its length IS the element count (unlike Set/Map.len, which
          -- would need dedup — deferred). Non-list operand / unmodelable arg → cannotProve.
          (match children[1]? with
           | some cId =>
             (match symEval m senv fuel ty cId with
              | .sym (.ctor t elems) =>
                if t == "list".toUTF8 then .sym (.const (.int (Int.ofNat elems.size)))
                else .cannotProve "symeval: List.len on a non-list value"
              | .cannotProve r => .cannotProve r
              | _ => .cannotProve "symeval: List.len on a non-list value")
           | none => .cannotProve "symeval: malformed List.len")
        else if q == "List".toUTF8 && mem == "push".toUTF8 then
          -- `List.push lst x` over a concrete list → the SAME list with `x` appended (`.ctor "list"`
          -- with the element SymExpr pushed). SOUND: `evalNode`'s `List.push` is exactly `es.push x`
          -- (order-preserving append of one element, #1607-1614). Non-list operand / unmodelable `x`
          -- → cannotProve (conservative; symbolic path never poisons).
          (match children[1]?, children[2]? with
           | some lId, some xId =>
             (match symEval m senv fuel ty lId, symEval m senv fuel ty xId with
              | .sym (.ctor t elems), .sym xe =>
                if t == "list".toUTF8 then .sym (.ctor "list".toUTF8 (elems.push xe))
                else .cannotProve "symeval: List.push on a non-list value"
              | .cannotProve r, _ => .cannotProve r
              | _, .cannotProve r => .cannotProve r
              | _, _ => .cannotProve "symeval: List.push on a non-list value")
           | _, _ => .cannotProve "symeval: malformed List.push")
        else if q == "List".toUTF8 && mem == "prepend".toUTF8 then
          -- `List.prepend lst x` → `x` CONSED AT THE FRONT (`.ctor "list" (#[x] ++ elems)`): the front
          -- analogue of `List.push` (append). Lists are ORDERED + strict, so no canonicalization — index 0
          -- is the prepended element. Closes v-cdz-smith's #7513 List.prepend boundary. Non-list operand /
          -- unmodelable `x` → cannotProve.
          (match children[1]?, children[2]? with
           | some lId, some xId =>
             (match symEval m senv fuel ty lId, symEval m senv fuel ty xId with
              | .sym (.ctor t elems), .sym xe =>
                if t == "list".toUTF8 then .sym (.ctor "list".toUTF8 (#[xe] ++ elems))
                else .cannotProve "symeval: List.prepend on a non-list value"
              | .cannotProve r, _ => .cannotProve r
              | _, .cannotProve r => .cannotProve r
              | _, _ => .cannotProve "symeval: List.prepend on a non-list value")
           | _, _ => .cannotProve "symeval: malformed List.prepend")
        else if q == "List".toUTF8 && (mem == "at".toUTF8 || mem == "get".toUTF8) then
          -- `List.at lst i` / `List.get lst i` (concrete list + concrete int index) → `Some lst[i]` when
          -- `0 ≤ i < len`, else `None` — byte-faithful to `evalNode` (Eval.lean:1737-1745). PURELY
          -- STRUCTURAL: no key-equality decision (unlike Set.contains/Map.lookup, which need bit-faithful
          -- `cmpValue`, NOT SymExpr-beq) and no dedup. A symbolic (non-const) index → cannotProve.
          (match children[1]?, children[2]? with
           | some lId, some iId =>
             (match symEval m senv fuel ty lId, symEval m senv fuel ty iId with
              | .sym (.ctor t elems), .sym (.const (.int i)) =>
                if t == "list".toUTF8 then
                  (if 0 ≤ i && i < Int.ofNat elems.size then
                     (match elems[i.toNat]? with
                      | some e => .sym (.ctor "Some".toUTF8 #[e])
                      | none => .sym (.ctor "None".toUTF8 #[]))
                   else .sym (.ctor "None".toUTF8 #[]))
                else .cannotProve "symeval: List.at on a non-list value"
              | .cannotProve r, _ => .cannotProve r
              | _, .cannotProve r => .cannotProve r
              | _, _ => .cannotProve "symeval: List.at on non-list / non-const-int index")
           | _, _ => .cannotProve "symeval: malformed List.at")
        else if (q == "String".toUTF8 && mem == "concat".toUTF8) then
          -- `String.concat a b` over two concrete strings → the concatenated string `.const (.str (x ++ y))`,
          -- byte-faithful to `evalNode` (Eval.lean:1754-1756). PURELY STRUCTURAL over scalar const leaves
          -- (no equality/dedup/float). Non-string operand / unmodelable arg → cannotProve.
          (match children[1]?, children[2]? with
           | some aId, some bId =>
             (match symEval m senv fuel ty aId, symEval m senv fuel ty bId with
              | .sym (.const (.str x)), .sym (.const (.str y)) => .sym (.const (.str (x ++ y)))
              | .cannotProve r, _ => .cannotProve r
              | _, .cannotProve r => .cannotProve r
              | _, _ => .cannotProve "symeval: String.concat on non-string values")
           | _, _ => .cannotProve "symeval: malformed String.concat")
        else if (q == "Bytes".toUTF8 && mem == "concat".toUTF8) then
          -- `Bytes.concat a b` over two concrete byte-strings → `.const (.bytes (x ++ y))`, byte-faithful to
          -- `evalNode` (Eval.lean:1615-1617). Same purely-structural scalar-concat shape as String.concat.
          (match children[1]?, children[2]? with
           | some aId, some bId =>
             (match symEval m senv fuel ty aId, symEval m senv fuel ty bId with
              | .sym (.const (.bytes x)), .sym (.const (.bytes y)) => .sym (.const (.bytes (x ++ y)))
              | .cannotProve r, _ => .cannotProve r
              | _, .cannotProve r => .cannotProve r
              | _, _ => .cannotProve "symeval: Bytes.concat on non-bytes values")
           | _, _ => .cannotProve "symeval: malformed Bytes.concat")
        else if q == "Set".toUTF8 && mem == "contains".toUTF8 then
          -- `Set.contains s x` → `.const (.bool (x ∈ s))`, mirroring `evalNode` (Eval.lean:1648-1650,
          -- `es.any (valEq · x)`). Membership is dup/order-INVARIANT, so the source-order `.ctor "set"`
          -- is faithful WITHOUT canonicalization (unlike Set.len, which needs dedup — deferred). 🪤 decide
          -- equality with BIT-FAITHFUL `valEq`/`cmpValue` (NaN dedupes, +0.0 ≠ -0.0), NEVER SymExpr-beq —
          -- whose IEEE float `==` (+0.0 == -0.0, NaN ≠ NaN) is the OPPOSITE and would be unsound. Only when
          -- EVERY element AND the query are concrete consts (a symbolic element could coincide at runtime →
          -- can't soundly conclude ∉); otherwise cannotProve.
          (match children[1]?, children[2]? with
           | some sId, some xId =>
             (match symEval m senv fuel ty sId, symEval m senv fuel ty xId with
              | .sym (.ctor t elems), .sym xe =>
                if t == "set".toUTF8 then
                  -- reify EVERY element AND the query to Values (incl COMPOUND list/tuple/record via
                  -- `symElemToValue?`); membership via bit-faithful `valEq` (NaN dedupes, +0.0 ≠ -0.0 —
                  -- NEVER SymExpr-beq). Result is a bool → no set rebuild. A symbolic element/query → cannotProve.
                  (match elems.mapM symElemToValue?, symElemToValue? xe with
                   | some vals, some xval => .sym (.const (.bool (vals.any (fun ev => valEq ev xval))))
                   | _, _ => .cannotProve "symeval: Set.contains needs all-concrete elements")
                else .cannotProve "symeval: Set.contains on a non-set value"
              | .cannotProve r, _ => .cannotProve r
              | _, .cannotProve r => .cannotProve r
              | _, _ => .cannotProve "symeval: Set.contains on non-set / non-value query")
           | _, _ => .cannotProve "symeval: malformed Set.contains")
        else if q == "String".toUTF8 && mem == "byte-len".toUTF8 then
          -- `String.byte-len s` over a concrete string → the UTF-8 BYTE count (`.const (.int b.size)`),
          -- byte-faithful to `evalNode` (Eval.lean:1761-1762). Purely structural. Non-string → cannotProve.
          (match children[1]? with
           | some sId =>
             (match symEval m senv fuel ty sId with
              | .sym (.const (.str b)) => .sym (.const (.int (Int.ofNat b.size)))
              | .cannotProve r => .cannotProve r
              | _ => .cannotProve "symeval: String.byte-len on a non-string value")
           | none => .cannotProve "symeval: malformed String.byte-len")
        else if q == "String".toUTF8 && mem == "scalar-len".toUTF8 then
          -- `String.scalar-len s` → the Unicode SCALAR (code-point) count, `evalNode` decodes UTF-8 then
          -- `s.toList.length` (Eval.lean:1764-1766); invalid UTF-8 traps `.unsupported` → cannotProve here.
          (match children[1]? with
           | some sId =>
             (match symEval m senv fuel ty sId with
              | .sym (.const (.str b)) =>
                (match String.fromUTF8? b with
                 | some s => .sym (.const (.int (Int.ofNat s.toList.length)))
                 | none => .cannotProve "symeval: String.scalar-len on invalid UTF-8")
              | .cannotProve r => .cannotProve r
              | _ => .cannotProve "symeval: String.scalar-len on a non-string value")
           | none => .cannotProve "symeval: malformed String.scalar-len")
        else if q == "String".toUTF8 && mem == "slice".toUTF8 then
          -- `String.slice s start end` (3-arg, SCALAR-indexed substring) → `Some s[start..end)` when
          -- `0 ≤ start ≤ end ≤ scalar-count`, else `None` — byte-faithful to `evalNode` (Eval.lean:1680-1692):
          -- decode UTF-8, take code-points `[start, end)`, re-encode. Invalid UTF-8 / non-const args →
          -- cannotProve. `String.ofList` ≡ eval's `String.mk` (same List Char → String), so byte-identical.
          (match children[1]?, children[2]?, children[3]? with
           | some sId, some startId, some endId =>
             (match symEval m senv fuel ty sId, symEval m senv fuel ty startId, symEval m senv fuel ty endId with
              | .sym (.const (.str b)), .sym (.const (.int start)), .sym (.const (.int «end»)) =>
                (match String.fromUTF8? b with
                 | some s =>
                   let cs := s.toList
                   if 0 ≤ start && start ≤ «end» && «end» ≤ Int.ofNat cs.length then
                     .sym (.ctor "Some".toUTF8
                       #[.const (.str (String.toUTF8 (String.ofList ((cs.drop start.toNat).take («end».toNat - start.toNat)))))])
                   else .sym (.ctor "None".toUTF8 #[])
                 | none => .cannotProve "symeval: String.slice on invalid UTF-8")
              | .cannotProve r, _, _ => .cannotProve r
              | _, .cannotProve r, _ => .cannotProve r
              | _, _, .cannotProve r => .cannotProve r
              | _, _, _ => .cannotProve "symeval: String.slice on non-string / non-const indices")
           | _, _, _ => .cannotProve "symeval: malformed String.slice")
        else if q == "String".toUTF8 && mem == "at".toUTF8 then
          -- `String.at s i` — Unicode-SCALAR-indexed char access → `Some s[i]` (a single-char string) when
          -- `0 ≤ i < char-count`, else `None`; byte-faithful to evalNode (Eval.lean:1697-1706): decode UTF-8,
          -- take code-point `i`, re-encode as a 1-char string. Invalid UTF-8 / non-const args → cannotProve.
          (match children[1]?, children[2]? with
           | some sId, some iId =>
             (match symEval m senv fuel ty sId, symEval m senv fuel ty iId with
              | .sym (.const (.str bytes)), .sym (.const (.int i)) =>
                (match String.fromUTF8? bytes with
                 | some s =>
                   let cs := s.toList
                   if 0 ≤ i && i < Int.ofNat cs.length then
                     (match cs[i.toNat]? with
                      | some ch => .sym (.ctor "Some".toUTF8 #[.const (.str (String.toUTF8 ch.toString))])
                      | none => .sym (.ctor "None".toUTF8 #[]))
                   else .sym (.ctor "None".toUTF8 #[])
                 | none => .cannotProve "symeval: String.at on invalid UTF-8")
              | .cannotProve r, _ => .cannotProve r
              | _, .cannotProve r => .cannotProve r
              | _, _ => .cannotProve "symeval: String.at on non-string / non-const index")
           | _, _ => .cannotProve "symeval: malformed String.at")
        else if q == "String".toUTF8 && mem == "scalar-at".toUTF8 then
          -- `String.scalar-at s i` — Unicode-SCALAR-indexed access → `Some <char>` (an Option<Char>, the
          -- CHAR value) when `0 ≤ i < scalar-count`, else `None`. Same indexing as `String.at` but yields
          -- `.char` not a single-char string; byte-faithful to evalNode. (v-cdz-smith next-tier boundary.)
          (match children[1]?, children[2]? with
           | some sId, some iId =>
             (match symEval m senv fuel ty sId, symEval m senv fuel ty iId with
              | .sym (.const (.str bytes)), .sym (.const (.int i)) =>
                (match String.fromUTF8? bytes with
                 | some s =>
                   let cs := s.toList
                   if 0 ≤ i && i < Int.ofNat cs.length then
                     (match cs[i.toNat]? with
                      | some ch => .sym (.ctor "Some".toUTF8 #[.const (.char (String.toUTF8 ch.toString))])
                      | none => .sym (.ctor "None".toUTF8 #[]))
                   else .sym (.ctor "None".toUTF8 #[])
                 | none => .cannotProve "symeval: String.scalar-at on invalid UTF-8")
              | .cannotProve r, _ => .cannotProve r
              | _, .cannotProve r => .cannotProve r
              | _, _ => .cannotProve "symeval: String.scalar-at on non-string / non-const index")
           | _, _ => .cannotProve "symeval: malformed String.scalar-at")
        else if q == "Bytes".toUTF8 && mem == "len".toUTF8 then
          -- `Bytes.len b` → the byte count (`.const (.int b.size)`), byte-faithful to evalNode
          -- (Eval.lean:1711-1712). Purely structural. Non-bytes → cannotProve.
          (match children[1]? with
           | some bId =>
             (match symEval m senv fuel ty bId with
              | .sym (.const (.bytes b)) => .sym (.const (.int (Int.ofNat b.size)))
              | .cannotProve r => .cannotProve r
              | _ => .cannotProve "symeval: Bytes.len on a non-bytes value")
           | none => .cannotProve "symeval: malformed Bytes.len")
        else if q == "Bytes".toUTF8 && mem == "at".toUTF8 then
          -- `Bytes.at b i` (byte-indexed) → `Some (Int b[i])` when `0 ≤ i < len`, else `None` — byte-faithful
          -- to evalNode (Eval.lean:1728-1732; the byte is widened UInt8→Int). Non-const index → cannotProve.
          (match children[1]?, children[2]? with
           | some bId, some iId =>
             (match symEval m senv fuel ty bId, symEval m senv fuel ty iId with
              | .sym (.const (.bytes b)), .sym (.const (.int i)) =>
                (if 0 ≤ i && i < Int.ofNat b.size then
                   (match b[i.toNat]? with
                    | some byte => .sym (.ctor "Some".toUTF8 #[.const (.int (Int.ofNat byte.toNat))])
                    | none => .sym (.ctor "None".toUTF8 #[]))
                 else .sym (.ctor "None".toUTF8 #[]))
              | .cannotProve r, _ => .cannotProve r
              | _, .cannotProve r => .cannotProve r
              | _, _ => .cannotProve "symeval: Bytes.at on non-bytes / non-const index")
           | _, _ => .cannotProve "symeval: malformed Bytes.at")
        else if q == "Bytes".toUTF8 && mem == "slice".toUTF8 then
          -- `Bytes.slice b start LENGTH` (byte-indexed, start/length — NOT start/end) → `Some b[start..start+len)`
          -- when `0 ≤ start ∧ 0 ≤ len ∧ start+len ≤ size`, else `None` — byte-faithful (Eval.lean:1714-1723).
          (match children[1]?, children[2]?, children[3]? with
           | some bId, some startId, some lenId =>
             (match symEval m senv fuel ty bId, symEval m senv fuel ty startId, symEval m senv fuel ty lenId with
              | .sym (.const (.bytes b)), .sym (.const (.int start)), .sym (.const (.int len)) =>
                (if 0 ≤ start && 0 ≤ len && start + len ≤ Int.ofNat b.size then
                   .sym (.ctor "Some".toUTF8 #[.const (.bytes (b.extract start.toNat (start.toNat + len.toNat)))])
                 else .sym (.ctor "None".toUTF8 #[]))
              | .cannotProve r, _, _ => .cannotProve r
              | _, .cannotProve r, _ => .cannotProve r
              | _, _, .cannotProve r => .cannotProve r
              | _, _, _ => .cannotProve "symeval: Bytes.slice on non-bytes / non-const args")
           | _, _, _ => .cannotProve "symeval: malformed Bytes.slice")
        else if q == "Set".toUTF8 && mem == "len".toUTF8 then
          -- `Set.len s` → distinct-element count. 🔑 FAITHFUL via eval's OWN `canonSet`: eval canonicalizes
          -- the set at literal construction (sort + dedup), so its `.len` is the canonical size; I reify the
          -- source-order const elements and run the SAME `canonSet` → identical size. Unorderable element
          -- (canonSet `none`) → cannotProve (eval would have declined the literal too). Non-const → cannotProve.
          (match children[1]? with
           | some sId =>
             (match symEval m senv fuel ty sId with
              | .sym (.ctor t elems) =>
                if t == "set".toUTF8 then
                  -- reify EVERY concrete element to a Value (incl COMPOUND list/tuple/record via
                  -- `symElemToValue?`) then run eval's own `canonSet` (sort + structural dedup via `cmpValue`,
                  -- which orders compounds lexicographically) → the canonical distinct-count. A symbolic /
                  -- unorderable / non-list-ctor element → cannotProve (eval would decline the literal too).
                  (match elems.mapM symElemToValue? with
                   | some vs => (match canonSet vs with
                                 | some s => .sym (.const (.int (Int.ofNat s.size)))
                                 | none => .cannotProve "symeval: Set.len on unorderable elements")
                   | none => .cannotProve "symeval: Set.len needs all-concrete elements")
                else .cannotProve "symeval: Set.len on a non-set value"
              | .cannotProve r => .cannotProve r
              | _ => .cannotProve "symeval: Set.len on a non-set value")
           | none => .cannotProve "symeval: malformed Set.len")
        else if q == "Map".toUTF8 && mem == "len".toUTF8 then
          -- `Map.len mp` → unique-key count. FAITHFUL via eval's OWN `canonMap` (last-insert-wins per key +
          -- sort). Entries are `.tuple #[k, v]`; reify all-const (key,val) pairs, run canonMap, take size.
          (match children[1]? with
           | some mId =>
             (match symEval m senv fuel ty mId with
              | .sym (.ctor t elems) =>
                if t == "map".toUTF8 then
                  -- reify each `.tuple #[k, v]` entry to concrete (key, value) Values (COMPOUND keys/values
                  -- via `symElemToValue?`), then eval's `canonMap` (last-insert-wins per key + sort) → the
                  -- canonical unique-KEY count. A symbolic / unorderable / malformed entry → cannotProve.
                  (match elems.mapM (fun e => match e with
                                              | .tuple #[ke, ve] => (match symElemToValue? ke, symElemToValue? ve with
                                                                     | some k, some v => some (k, v)
                                                                     | _, _ => none)
                                              | _ => none) with
                   | some kvs => (match canonMap kvs with
                                  | some cm => .sym (.const (.int (Int.ofNat cm.size)))
                                  | none => .cannotProve "symeval: Map.len on unorderable key")
                   | none => .cannotProve "symeval: Map.len needs all-concrete entries")
                else .cannotProve "symeval: Map.len on a non-map value"
              | .cannotProve r => .cannotProve r
              | _ => .cannotProve "symeval: Map.len on a non-map value")
           | none => .cannotProve "symeval: malformed Map.len")
        else if q == "Set".toUTF8 && mem == "to-list".toUTF8 then
          -- `Set.to-list s` → the ordered list of the set's CANONICAL (sorted, deduped) elements as a
          -- `.ctor "list"` of `valueToSym`. Mirrors `Set.len` but returns the list VALUE, not the count —
          -- the value companion draining v-cdz-smith's #7412 to-list boundary. Non-set/symbolic/unorderable
          -- → cannotProve. (Canonical order matches a set's canonical value repr, so `List.at`/comparison over
          -- the result is faithful.)
          (match children[1]? with
           | some sId =>
             (match symEval m senv fuel ty sId with
              | .sym (.ctor t elems) =>
                if t == "set".toUTF8 then
                  (match elems.mapM symElemToValue? with
                   | some vs => (match canonSet vs with
                                 | some s => .sym (.ctor "list".toUTF8 (s.map valueToSym))
                                 | none => .cannotProve "symeval: Set.to-list on unorderable elements")
                   | none => .cannotProve "symeval: Set.to-list needs all-concrete elements")
                else .cannotProve "symeval: Set.to-list on a non-set value"
              | .cannotProve r => .cannotProve r
              | _ => .cannotProve "symeval: Set.to-list on a non-set value")
           | none => .cannotProve "symeval: malformed Set.to-list")
        else if q == "Map".toUTF8 && mem == "to-list".toUTF8 then
          -- `Map.to-list mp` → the ordered list of `(k, v)` tuples over the CANONICAL (last-write-wins,
          -- sorted-by-key) map, as a `.ctor "list"` of `.tuple #[.const k, .const v]`. Mirrors `Map.len` but
          -- returns the list VALUE. Non-map/symbolic/unorderable/malformed entry → cannotProve.
          (match children[1]? with
           | some mId =>
             (match symEval m senv fuel ty mId with
              | .sym (.ctor t elems) =>
                if t == "map".toUTF8 then
                  (match elems.mapM (fun e => match e with
                                              | .tuple #[ke, ve] => (match symElemToValue? ke, symElemToValue? ve with
                                                                     | some k, some v => some (k, v)
                                                                     | _, _ => none)
                                              | _ => none) with
                   | some kvs => (match canonMap kvs with
                                  | some cm => .sym (.ctor "list".toUTF8 (cm.map (fun kv => SymExpr.tuple #[.const kv.1, .const kv.2])))
                                  | none => .cannotProve "symeval: Map.to-list on unorderable key")
                   | none => .cannotProve "symeval: Map.to-list needs all-concrete entries")
                else .cannotProve "symeval: Map.to-list on a non-map value"
              | .cannotProve r => .cannotProve r
              | _ => .cannotProve "symeval: Map.to-list on a non-map value")
           | none => .cannotProve "symeval: malformed Map.to-list")
        else if q == "Map".toUTF8 && mem == "lookup".toUTF8 then
          -- `Map.lookup mp k` → `Some v` (k's value) or `None`, mirroring `evalNode`'s find over the
          -- CANONICALIZED map (Eval.lean:1664-1667, `es.find? (valEq ·.1 k) |>.map (·.2)`). 🔑 must `canonMap`
          -- FIRST so a DUP key resolves LAST-insert-wins (source-order first-match would diverge). The result
          -- is a VALUE (not a map structure) → no source/canonical order-consistency concern. Equality via
          -- BIT-FAITHFUL valEq. All-const entries + const key required; else cannotProve.
          (match children[1]?, children[2]? with
           | some mId, some kId =>
             (match symEval m senv fuel ty mId, symEval m senv fuel ty kId with
              | .sym (.ctor t elems), .sym (.const kq) =>
                if t == "map".toUTF8 then
                  (match elems.mapM (fun e => match e with
                                              | .tuple #[.const k, .const v] => some (k, v)
                                              | _ => none) with
                   | some kvs =>
                     (match canonMap kvs with
                      | some cm => (match (cm.find? (fun kv => valEq kv.1 kq)).map (·.2) with
                                    | some v => .sym (.ctor "Some".toUTF8 #[.const v])
                                    | none => .sym (.ctor "None".toUTF8 #[]))
                      | none => .cannotProve "symeval: Map.lookup on unorderable key")
                   | none => .cannotProve "symeval: Map.lookup needs all-concrete entries")
                else .cannotProve "symeval: Map.lookup on a non-map value"
              | .cannotProve r, _ => .cannotProve r
              | _, .cannotProve r => .cannotProve r
              | _, _ => .cannotProve "symeval: Map.lookup on non-map / non-const key")
           | _, _ => .cannotProve "symeval: malformed Map.lookup")
        else if q == "Set".toUTF8 && mem == "of".toUTF8 then
          -- `Set.of (list e…)` → the canonical set of the list's elements (`canonSet elems`, byte-faithful to
          -- `evalSetOf`, Eval.lean:1499-1510). The set-CONSTRUCTION companion of the `(set …)` literal +
          -- Set.insert/union. 🪤🪤 v-cdz-smith #7371/#7412 residual root-cause (raw-hex repro, this PR): a
          -- SOURCE program building a set via `Set.of` sank to cannotProve (Set.of was UNMODELED) while its
          -- `--target cadenza` ROUNDTRIP const-folds to a `setCtor`-headed literal that ALREADY folds
          -- (`headName?` maps the setCtor leaf kind → "set" → the set-literal handler) → the whole Set-op
          -- chain boundaried in the differential. The gap was the SOURCE `Set.of`, NOT the roundtrip literal
          -- head (which needed no fix). Arg must be a concrete `.ctor "list"` of all-concrete (incl compound
          -- via `symElemToValue?`) elements; a symbolic/unorderable element → cannotProve.
          (match children[1]? with
           | some listId =>
             (match symEval m senv fuel ty listId with
              | .sym (.ctor t elems) =>
                if t == "list".toUTF8 then
                  (match elems.mapM symElemToValue? with
                   | some vals => (match canonSet vals with
                                   | some s => .sym (.ctor "set".toUTF8 (s.map valueToSym))
                                   | none => .cannotProve "symeval: Set.of on unorderable elements")
                   | none => .cannotProve "symeval: Set.of needs all-concrete list elements")
                else .cannotProve "symeval: Set.of argument is not a list value"
              | .cannotProve r => .cannotProve r
              | _ => .cannotProve "symeval: Set.of argument is not a list value")
           | none => .cannotProve "symeval: malformed Set.of")
        else if q == "Set".toUTF8 && mem == "insert".toUTF8 then
          -- `Set.insert s x` → the set with `x` added, RE-CANONICALIZED (`canonSet (es.push x)`,
          -- Eval.lean:1655-1659). Now that set LITERALS are canonical too (#6691), this canonical output is
          -- consistent with them. All-const elements + const `x` required; unorderable → cannotProve.
          (match children[1]?, children[2]? with
           | some sId, some xId =>
             (match symEval m senv fuel ty sId, symEval m senv fuel ty xId with
              | .sym (.ctor t elems), .sym xe =>
                if t == "set".toUTF8 then
                  -- reify EVERY element AND the inserted value to Values (incl COMPOUND list/tuple/record via
                  -- `symElemToValue?`), `canonSet` the pushed array (sort + structural dedup), then REBUILD the
                  -- canonical set as SymExprs via `valueToSym` (representation-faithful → matches a set literal
                  -- of the same elements). A symbolic / unorderable element → cannotProve.
                  (match elems.mapM symElemToValue?, symElemToValue? xe with
                   | some vals, some xval =>
                     (match canonSet (vals.push xval) with
                      | some s => .sym (.ctor "set".toUTF8 (s.map valueToSym))
                      | none => .cannotProve "symeval: Set.insert on unorderable elements")
                   | _, _ => .cannotProve "symeval: Set.insert needs all-concrete elements")
                else .cannotProve "symeval: Set.insert on a non-set value"
              | .cannotProve r, _ => .cannotProve r
              | _, .cannotProve r => .cannotProve r
              | _, _ => .cannotProve "symeval: Set.insert on non-set / non-value element")
           | _, _ => .cannotProve "symeval: malformed Set.insert")
        else if q == "Set".toUTF8 && mem == "union".toUTF8 then
          -- `Set.union a b` → the dedup-MERGED canonical set (`canonSet (va ++ vb)`) — the Set companion of
          -- `Set.insert`, closing the v-cdz-smith #7371 set-transform boundary. Both operands must be concrete
          -- `.ctor "set"` sets (COMPOUND elements via `symElemToValue?`); unorderable/symbolic → cannotProve.
          (match children[1]?, children[2]? with
           | some aId, some bId =>
             (match symEval m senv fuel ty aId, symEval m senv fuel ty bId with
              | .sym (.ctor ta ea), .sym (.ctor tb eb) =>
                if ta == "set".toUTF8 && tb == "set".toUTF8 then
                  (match ea.mapM symElemToValue?, eb.mapM symElemToValue? with
                   | some va, some vb =>
                     (match canonSet (va ++ vb) with
                      | some s => .sym (.ctor "set".toUTF8 (s.map valueToSym))
                      | none => .cannotProve "symeval: Set.union on unorderable elements")
                   | _, _ => .cannotProve "symeval: Set.union needs all-concrete elements")
                else .cannotProve "symeval: Set.union on a non-set value"
              | .cannotProve r, _ => .cannotProve r
              | _, .cannotProve r => .cannotProve r
              | _, _ => .cannotProve "symeval: Set.union on non-set operands")
           | _, _ => .cannotProve "symeval: malformed Set.union")
        else if q == "Set".toUTF8 && mem == "intersection".toUTF8 then
          -- `Set.intersection a b` → the canonical set of elements in BOTH `a` and `b`
          -- (`canonSet (va.filter (∈ vb))`). Prelude type `∀a. (Set a) → (Set a) → (Set a)`
          -- (prelude.rs:714/745); COMMUTATIVE. The Set companion of `union`, closing v-cdz-smith's #7506
          -- intersection/difference boundary (the last Set-op residual after #7507's Set.of). Membership
          -- via BIT-FAITHFUL `valEq` (NaN dedupes, +0.0 ≠ -0.0 — NEVER SymExpr-beq, whose IEEE `==` is the
          -- opposite and unsound), mirroring `Set.contains`. Both operands concrete `.ctor "set"` (COMPOUND
          -- elements via `symElemToValue?`); symbolic/unorderable → cannotProve.
          (match children[1]?, children[2]? with
           | some aId, some bId =>
             (match symEval m senv fuel ty aId, symEval m senv fuel ty bId with
              | .sym (.ctor ta ea), .sym (.ctor tb eb) =>
                if ta == "set".toUTF8 && tb == "set".toUTF8 then
                  (match ea.mapM symElemToValue?, eb.mapM symElemToValue? with
                   | some va, some vb =>
                     (match canonSet (va.filter (fun v => vb.any (fun w => valEq w v))) with
                      | some s => .sym (.ctor "set".toUTF8 (s.map valueToSym))
                      | none => .cannotProve "symeval: Set.intersection on unorderable elements")
                   | _, _ => .cannotProve "symeval: Set.intersection needs all-concrete elements")
                else .cannotProve "symeval: Set.intersection on a non-set value"
              | .cannotProve r, _ => .cannotProve r
              | _, .cannotProve r => .cannotProve r
              | _, _ => .cannotProve "symeval: Set.intersection on non-set operands")
           | _, _ => .cannotProve "symeval: malformed Set.intersection")
        else if q == "Set".toUTF8 && mem == "difference".toUTF8 then
          -- `Set.difference a b` → the canonical set of elements in `a` but NOT in `b` (`a \ b`,
          -- `canonSet (va.filter (∉ vb))`). Prelude type `∀a. (Set a) → (Set a) → (Set a)`; NON-COMMUTATIVE
          -- (`a` is the minuend, `b` the subtrahend — arg ORDER is load-bearing, a swap = a FALSE verdict).
          -- Same bit-faithful `valEq` membership + concrete-operand discipline as `intersection`.
          (match children[1]?, children[2]? with
           | some aId, some bId =>
             (match symEval m senv fuel ty aId, symEval m senv fuel ty bId with
              | .sym (.ctor ta ea), .sym (.ctor tb eb) =>
                if ta == "set".toUTF8 && tb == "set".toUTF8 then
                  (match ea.mapM symElemToValue?, eb.mapM symElemToValue? with
                   | some va, some vb =>
                     (match canonSet (va.filter (fun v => !(vb.any (fun w => valEq w v)))) with
                      | some s => .sym (.ctor "set".toUTF8 (s.map valueToSym))
                      | none => .cannotProve "symeval: Set.difference on unorderable elements")
                   | _, _ => .cannotProve "symeval: Set.difference needs all-concrete elements")
                else .cannotProve "symeval: Set.difference on a non-set value"
              | .cannotProve r, _ => .cannotProve r
              | _, .cannotProve r => .cannotProve r
              | _, _ => .cannotProve "symeval: Set.difference on non-set operands")
           | _, _ => .cannotProve "symeval: malformed Set.difference")
        else if q == "Set".toUTF8 && mem == "remove".toUTF8 then
          -- `Set.remove s x` → the set minus `x` (bit-faithful `valEq`, mirroring `Set.contains`/`Map.remove`),
          -- re-canonicalized. Concrete set + value required; symbolic/unorderable → cannotProve.
          (match children[1]?, children[2]? with
           | some sId, some xId =>
             (match symEval m senv fuel ty sId, symEval m senv fuel ty xId with
              | .sym (.ctor t elems), .sym xe =>
                if t == "set".toUTF8 then
                  (match elems.mapM symElemToValue?, symElemToValue? xe with
                   | some vals, some xval =>
                     (match canonSet (vals.filter (fun v => !(valEq v xval))) with
                      | some s => .sym (.ctor "set".toUTF8 (s.map valueToSym))
                      | none => .cannotProve "symeval: Set.remove on unorderable elements")
                   | _, _ => .cannotProve "symeval: Set.remove needs all-concrete elements")
                else .cannotProve "symeval: Set.remove on a non-set value"
              | .cannotProve r, _ => .cannotProve r
              | _, .cannotProve r => .cannotProve r
              | _, _ => .cannotProve "symeval: Set.remove on non-set / non-value element")
           | _, _ => .cannotProve "symeval: malformed Set.remove")
        else if q == "Map".toUTF8 && mem == "remove".toUTF8 then
          -- `Map.remove mp k` → the map without k's entry, re-canonicalized (`canonMap (es.filter (·.1 ≠ k))`,
          -- Eval.lean:1746-1749). Reify all-const `.tuple #[k,v]` entries, drop keys valEq `k`, canonMap.
          (match children[1]?, children[2]? with
           | some mId, some kId =>
             (match symEval m senv fuel ty mId, symEval m senv fuel ty kId with
              | .sym (.ctor t elems), .sym (.const kv) =>
                if t == "map".toUTF8 then
                  (match elems.mapM (fun e => match e with
                                              | .tuple #[.const k, .const v] => some (k, v)
                                              | _ => none) with
                   | some kvs =>
                     (match canonMap (kvs.filter (fun e => !(valEq e.1 kv))) with
                      | some cm => .sym (.ctor "map".toUTF8 (cm.map (fun p => SymExpr.tuple #[.const p.1, .const p.2])))
                      | none => .cannotProve "symeval: Map.remove on unorderable key")
                   | none => .cannotProve "symeval: Map.remove needs all-concrete entries")
                else .cannotProve "symeval: Map.remove on a non-map value"
              | .cannotProve r, _ => .cannotProve r
              | _, .cannotProve r => .cannotProve r
              | _, _ => .cannotProve "symeval: Map.remove on non-map / non-const key")
           | _, _ => .cannotProve "symeval: malformed Map.remove")
        else if q == "Map".toUTF8 && mem == "empty".toUTF8 then
          -- `(Map.empty)` = `((. Map empty))` (nullary) → the empty map VALUE (Eval.lean:1124 zero-arg
          -- call / 2063 projection). The map-building base for `Map.insert` chains (v-cdz-smith boundary):
          -- without this, `Map.insert (Map.insert Map.empty …)` sinks to cannotProve and `Map.lookup`/`Map.len`
          -- over it cannot fold. Distinct empty-map ctor consumed by the Map.insert/lookup/len/remove handlers.
          .sym (.ctor "map".toUTF8 #[])
        else if q == "Map".toUTF8 && mem == "insert".toUTF8 then
          -- `Map.insert mp k v` → `mp` with `k ↦ v` (LAST-write-wins on a dup key), re-canonicalized —
          -- byte-faithful to `evalMapInsert` (Eval.lean:1512-1529): `canonMap (mapInsertRaw es k v)` where
          -- `mapInsertRaw es k v = (es.filter (·.1 ≠ k)).push (k,v)`. Reify all-const `.tuple #[k,v]` entries
          -- + const key/value; a symbolic/unorderable/malformed operand → cannotProve. The Map twin of
          -- `Set.insert`; closes the `Map.insert (Map.insert Map.empty …)` construction boundary.
          (match children[1]?, children[2]?, children[3]? with
           | some mId, some kId, some vId =>
             (match symEval m senv fuel ty mId, symEval m senv fuel ty kId, symEval m senv fuel ty vId with
              | .sym (.ctor t elems), .sym (.const kk), .sym (.const vv) =>
                if t == "map".toUTF8 then
                  (match elems.mapM (fun e => match e with
                                              | .tuple #[.const k, .const v] => some (k, v)
                                              | _ => none) with
                   | some kvs =>
                     (match canonMap (mapInsertRaw kvs kk vv) with
                      | some cm => .sym (.ctor "map".toUTF8 (cm.map (fun p => SymExpr.tuple #[.const p.1, .const p.2])))
                      | none => .cannotProve "symeval: Map.insert on unorderable key")
                   | none => .cannotProve "symeval: Map.insert needs all-concrete entries")
                else .cannotProve "symeval: Map.insert on a non-map value"
              | .cannotProve r, _, _ => .cannotProve r
              | _, .cannotProve r, _ => .cannotProve r
              | _, _, .cannotProve r => .cannotProve r
              | _, _, _ => .cannotProve "symeval: Map.insert on non-map / non-const key or value")
           | _, _, _ => .cannotProve "symeval: malformed Map.insert")
        else if q == "Map".toUTF8 && mem == "merge".toUTF8 then
          -- `Map.merge a b` → the union of the two maps, with the RIGHT operand `b` WINNING on an
          -- overlapping key (LAST-writer, prelude.rs:891-893). `canonMap` is last-insert-wins per key, so
          -- `canonMap (aKvs ++ bKvs)` gives exactly b-wins — the Map analogue of List.concat / the value-
          -- position map spread `#map((= k v) (.. m))`. Closes v-cdz-smith's #7513 Map.merge boundary. Both
          -- operands concrete `.ctor "map"` of all-const `.tuple #[k,v]` entries; symbolic/unorderable →
          -- cannotProve. 🪤 arg order is load-bearing (b wins) — pinned by a #guard, like Set.difference.
          (match children[1]?, children[2]? with
           | some aId, some bId =>
             (match symEval m senv fuel ty aId, symEval m senv fuel ty bId with
              | .sym (.ctor ta ea), .sym (.ctor tb eb) =>
                if ta == "map".toUTF8 && tb == "map".toUTF8 then
                  (match ea.mapM (fun e => match e with | .tuple #[.const k, .const v] => some (k, v) | _ => none),
                         eb.mapM (fun e => match e with | .tuple #[.const k, .const v] => some (k, v) | _ => none) with
                   | some kvsA, some kvsB =>
                     (match canonMap (kvsA ++ kvsB) with
                      | some cm => .sym (.ctor "map".toUTF8 (cm.map (fun p => SymExpr.tuple #[.const p.1, .const p.2])))
                      | none => .cannotProve "symeval: Map.merge on unorderable key")
                   | _, _ => .cannotProve "symeval: Map.merge needs all-concrete entries")
                else .cannotProve "symeval: Map.merge on a non-map value"
              | .cannotProve r, _ => .cannotProve r
              | _, .cannotProve r => .cannotProve r
              | _, _ => .cannotProve "symeval: Map.merge on non-map operands")
           | _, _ => .cannotProve "symeval: malformed Map.merge")
        else if q == "Option".toUTF8 && mem == "expect".toUTF8 then
          -- `Option.expect o` unwraps `Some x` → x (evalNode uses observeShallow, which is identity on a
          -- non-poison value; a modeled symbolic payload is never poison, so → x). `None` traps with a custom
          -- message (not modeled) → cannotProve. Byte-faithful to evalNode (Eval.lean:1622-1627).
          (match children[1]? with
           | some oId =>
             (match symEval m senv fuel ty oId with
              | .sym (.ctor t #[x]) =>
                if t == "Some".toUTF8 then .sym x
                else .cannotProve "symeval: Option.expect on a non-Option (Ok/Err) ctor"
              | .cannotProve r => .cannotProve r
              | _ => .cannotProve "symeval: Option.expect on None / non-Option (trap-message not modeled)")
           | none => .cannotProve "symeval: malformed Option.expect")
        else if (q == "Float64".toUTF8 || q == "Float32".toUTF8) && (mem == "nan".toUTF8 || mem == "Infinity".toUTF8) then
          -- prelude float CONSTANTS in a CALL/member-head position `((. Float64|Float32 nan|Infinity) …)`
          -- (companion to the proj-form handling): a nullary constant, args (if any) irrelevant. Byte-faithful
          -- to evalNode (Eval.lean:2052-2056). This lets a float ORDERING with a nan/inf constant operand fold
          -- (foldConst? then folds `(<= Float64.nan 834.0)` via its asF64? path — NaN cmp → false). (v-cdz-smith.)
          if mem == "nan".toUTF8 then .sym (.const .floatNan) else .sym (.const (.floatInf false))
        else if q == "Rational".toUTF8 && mem == "of".toUTF8 then
          -- `Rational.of n d` → the normalized exact rational n/d (`mkRational`: sign-normalize + gcd-reduce);
          -- a ZERO denominator TRAPS (unreachable) → cannotProve. Byte-faithful to evalNode (Eval.lean:1635-1647).
          (match children[1]?, children[2]? with
           | some nId, some dId =>
             (match symEval m senv fuel ty nId, symEval m senv fuel ty dId with
              | .sym (.const (.int n)), .sym (.const (.int d)) =>
                (match mkRational n d with
                 | some v => .sym (.const v)
                 | none => .cannotProve "symeval: Rational.of with zero denominator (traps)")
              | .cannotProve r, _ => .cannotProve r
              | _, .cannotProve r => .cannotProve r
              | _, _ => .cannotProve "symeval: Rational.of on non-integer operands")
           | _, _ => .cannotProve "symeval: malformed Rational.of")
        else if q == "Bytes".toUTF8 && mem == "of".toUTF8 then
          -- `Bytes.of (list i…)` builds a Bytes value from a list of byte-valued ints (0..255); an element
          -- outside 0..255 or non-int → evalNode `.unsupported` → cannotProve. Byte-faithful (Eval.lean:1672-1679).
          (match children[1]? with
           | some lId =>
             (match symEval m senv fuel ty lId with
              | .sym (.ctor t elems) =>
                if t == "list".toUTF8 then
                  (match elems.mapM (fun e => match e with
                                              | .const (.int n) => if 0 ≤ n && n < 256 then some (UInt8.ofNat n.toNat) else none
                                              | _ => none) with
                   | some bytes => .sym (.const (.bytes (ByteArray.mk bytes)))
                   | none => .cannotProve "symeval: Bytes.of element not a 0..255 const int")
                else .cannotProve "symeval: Bytes.of on a non-list value"
              | .cannotProve r => .cannotProve r
              | _ => .cannotProve "symeval: Bytes.of on a non-list value")
           | none => .cannotProve "symeval: malformed Bytes.of")
        else if q == "Unit".toUTF8 && mem == "base".toUTF8 then
          -- `Unit.base n` → a base UNIT value, modeled as `.ctor "unit" #[n]` (n the base-unit name/bytes).
          -- (v-cdz-smith Qty widening #7282; a Qty is a (magnitude, unit) pair — see `Qty.of`/`Qty.value`.)
          (match children[1]? with
           | some nId => (match symEval m senv fuel ty nId with
                          | .sym e => .sym (.ctor "unit".toUTF8 #[e])
                          | .cannotProve r => .cannotProve r)
           | none => .cannotProve "symeval: malformed Unit.base")
        else if q == "Qty".toUTF8 && mem == "of".toUTF8 then
          -- `Qty.of mag unit` → a QUANTITY value `.ctor "qty" #[mag, unit]` (magnitude + unit). `Qty.value`
          -- extracts the magnitude; same-unit `+`/`-` and `*` fold the magnitude (in the arith path below).
          (match children[1]?, children[2]? with
           | some mId, some uId =>
             (match symEval m senv fuel ty mId, symEval m senv fuel ty uId with
              | .sym me, .sym ue => .sym (.ctor "qty".toUTF8 #[me, ue])
              | .cannotProve r, _ => .cannotProve r
              | _, .cannotProve r => .cannotProve r)
           | _, _ => .cannotProve "symeval: malformed Qty.of")
        else if q == "Qty".toUTF8 && mem == "value".toUTF8 then
          -- `Qty.value q` → the MAGNITUDE of a `Qty` (`.ctor "qty" #[mag, _]`). Non-qty operand → boundary.
          (match children[1]? with
           | some qId =>
             (match symEval m senv fuel ty qId with
              | .sym (.ctor t #[me, _]) =>
                if t == "qty".toUTF8 then .sym me else .cannotProve "symeval: Qty.value on a non-qty value"
              | .cannotProve r => .cannotProve r
              | _ => .cannotProve "symeval: Qty.value on a non-qty value")
           | none => .cannotProve "symeval: malformed Qty.value")
        else if (parseIntTyName? q).isSome && (mem == "of".toUTF8 || mem == "wrap".toUTF8) then
          -- INT-CONVERSION `(<IntTy>.of x)` / `.wrap x` (Int8/16/32/64 + UInt8/16/32/64) — byte-faithful to
          -- evalNode (Eval.lean:1783-1805): `.wrap` reinterprets x mod 2^w (total, two's-complement);
          -- `.of` is CHECKED — in-range → the value, OUT-OF-RANGE → TRAPS (unreachable) so it does NOT fold
          -- (stays symbolic → cannotProve, never a false-proven value). BigInt: identity (both). (v-cdz-smith
          -- steering: the int-conversion `.of` family is the biggest modelable boundary chunk, ~60 cases.)
          (match children[1]? with
           | some xId =>
             (match symEval m senv fuel ty xId with
              | .sym (.const (.int x)) =>
                let tty := (parseIntTyName? q).get!
                (match tty.width with
                 | .bits w =>
                   let modw : Int := (2 : Int) ^ w
                   if mem == "wrap".toUTF8 then
                     let p := ((x % modw) + modw) % modw
                     .sym (.const (.int (if tty.signed && p ≥ (2 : Int) ^ (w - 1) then p - modw else p)))
                   else
                     let lo : Int := if tty.signed then -((2 : Int) ^ (w - 1)) else 0
                     let hi : Int := if tty.signed then (2 : Int) ^ (w - 1) else (2 : Int) ^ w
                     if lo ≤ x && x < hi then .sym (.const (.int x))
                     else .cannotProve "symeval: <IntTy>.of out of target range (traps unreachable)"
                 | _ => .sym (.const (.int x)))  -- BigInt (or unknown width): identity
              | .cannotProve r => .cannotProve r
              | _ => .cannotProve "symeval: <IntTy>.of/wrap on a non-integer operand")
           | none => .cannotProve "symeval: malformed <IntTy> conversion")
        else .cannotProve "symeval: member-op head not modeled (boundary)"
      | none =>
        -- a NON-NAME APPLICATION head that itself evaluates to a CLOSURE — a CURRY CHAIN
        -- `(((pa3 41) 7) 38)` (the head `((pa3 41) 7)` is an application, not a bound name). symEval the
        -- head; if it is a `.localFn`, apply it to the remaining args with the SAME partial/full discipline
        -- as the name-head local-fn call above. Byte-faithful to eval (Eval.lean:1119-1128: symEval the
        -- computed head → applyClosure). A non-closure head / unmodelable head → cannotProve (safe boundary).
        match children[0]? with
        | some hid =>
          (match symEval m senv fuel ty hid with
           | .sym (.localFn lspecs lbodyId lcap) =>
             if fuel == 0 then .cannotProve "symeval: curry-chain apply fuel exhausted (recursion?)"
             else
               let nargs := children.size - 1
               if 0 < nargs && nargs < lspecs.size then
                 -- PARTIAL: extend the closure over the remaining params (same as the name-head partial case).
                 let capExt := ((lspecs.extract 0 nargs).zip (children.extract 1 children.size)).foldl
                   (fun (acc : Option (List (ByteArray × SymExpr))) p =>
                     match acc with
                     | none => none
                     | some cap =>
                       match paramSpec? m p.1 with
                       | some (pnm, _) => (match symEval m senv fuel ty p.2 with
                                           | .sym e => some (cap ++ [(pnm, e)])
                                           | .cannotProve _ => none)
                       | none => none) (some lcap)
                 match capExt with
                 | some newCap => .sym (.localFn (lspecs.extract nargs lspecs.size) lbodyId newCap)
                 | none => .cannotProve "symeval: a curried (chain) argument is unmodelable"
               else if lspecs.size != nargs then .cannotProve "symeval: curry-chain arity mismatch (over-application)"
               else
                 let callEnv := (lspecs.zip (children.extract 1 children.size)).foldl
                   (fun (acc : Option SymEnv) p =>
                     match acc with
                     | none => none
                     | some env =>
                       match paramSpec? m p.1 with
                       | some (pnm, _) => (match symEval m senv fuel ty p.2 with
                                           | .sym e => some ((pnm, e) :: env)
                                           | .cannotProve _ => none)
                       | none => none) (some lcap)
                 match callEnv with
                 | some ce => symEval m ce (fuel - 1) defaultIntTy lbodyId
                 | none => .cannotProve "symeval: a curry-chain argument is unmodelable"
           | .sym e =>
             -- a NULLARY application `(e)` (single child, NO args) is a GROUPING — identity: `(x)` = `x`.
             -- Mirrors eval (Eval.lean:1123-1126: a size-1 computed head returns its value). Skips the
             -- grouping layer, matching the compiler's `skip_grouping_up` (the oracle's version of the #7227
             -- bug — v-cdz-smith's regression-guard GROUPED magnitudes `(Qty.of (6) …)` hit this). A computed
             -- non-closure head APPLIED to args (size > 1) stays a boundary.
             if children.size == 1 then .sym e
             else .cannotProve "symeval: non-name head is not a closure (computed non-function head applied)"
           | .cannotProve r => .cannotProve r)
        | none => .cannotProve "symeval: empty application (no head)"
  | none => .cannotProve "symeval: node index out of range"

/-- Symbolically evaluate a `(let ((n v)…) body)`: bind the remaining `(name value)` pairs `ps`
sequentially (each `v` symEval'd in the env extended with the EARLIER bindings — let*), then `symEval`
the body. A binding whose value is unmodelable (`cannotProve`) sinks the whole `let` — SOUND-conservative:
that binding could be an eager/discarded one (a strict list/set/map ctor, or a `?`) whose trap we cannot
rule out, so we must not silently drop it and claim a value. -/
partial def symLet (m : Module) (senv : SymEnv) (fuel : Nat) (ty : IntTy) (ps : List Nat) (bodyId : Nat) : SymOutcome :=
  match ps with
  | [] => symEval m senv fuel ty bodyId
  | pid :: rest =>
    match m.nodes[pid]? with
    | some (Node.list pc) =>
      match pc[0]?, pc[1]? with
      | some nId, some vId =>
        match nameOf? m nId with
        | some nm =>
          match symEval m senv fuel ty vId with
          | .sym e => symLet m ((nm, e) :: senv) fuel ty rest bodyId
          | .cannotProve r => .cannotProve r
        | none => .cannotProve "symeval: let binding missing name"
      | _, _ => .cannotProve "symeval: malformed let binding"
    | _ => .cannotProve "symeval: let binding not a (name value) pair"

/-- Symbolically evaluate an inline `(do stmt… last)` EXPRESSION, mirroring `evalDo` (Eval.lean:1220-1260):
the block's value is the LAST item, evaluated under the env extended by the preceding statements.
A `(def x val)` (bare-atom target) binds `x` sequentially (let-like, `.sym` value; unmodelable → sink).
A NON-DEF statement is DISCARDED and its trap ELIDED (unobserved) — so it is SKIPPED (same env), faithful
to the pure-value oracle. A LOCAL FUNCTION def `(def (f params) body)` (list target) binds a CLOSURE in
`evalDo`; the symbolic env holds no closures → `cannotProve` (punt, conservative). Empty do → cannotProve. -/
partial def symDo (m : Module) (senv : SymEnv) (fuel : Nat) (ty : IntTy) (children : Array Nat) : SymOutcome :=
  let items := children.extract 1 children.size
  match items.back? with
  | none => .cannotProve "symeval: empty do"
  | some lastId =>
    let rec bind (senv : SymEnv) (js : List Nat) : Except SymOutcome SymEnv :=
      match js with
      | [] => .ok senv
      | j :: rest =>
        match asDef? m j with
        | some dc =>
          match dc[1]?, dc[dc.size - 1]? with
          | some targetId, some valId =>
            match nameOf? m targetId with
            | some nm =>
              -- value binding `(def x val)` — bind sequentially (later stmts see it), like `let`.
              (match symEval m senv fuel ty valId with
               | .sym e => bind ((nm, e) :: senv) rest
               | .cannotProve r => .error (.cannotProve r))
            | none =>
              -- a LOCAL FUNCTION def `(def (fname params) body)` (list target) — bind `fname` to a `localFn`
              -- carrying its param specs + body so a later `(fname args)` call INLINES it (the call path
              -- does params-only, no capture → sound; see there). fname = the target list's head name.
              (match m.nodes[targetId]? with
               | some tnode =>
                 (match m.headName? tnode with
                  | some fname => bind ((fname, SymExpr.localFn (paramSpecNodes m targetId) valId senv) :: senv) rest
                  | none => .error (.cannotProve "symeval: malformed do local-fn def target"))
               | none => .error (.cannotProve "symeval: malformed do local-fn def target"))
          | _, _ => .error (.cannotProve "symeval: malformed do def")
        -- a NON-DEF statement: value discarded, trap elided → skip, same env (faithful to evalDo).
        | none => bind senv rest
    match bind senv (items.extract 0 (items.size - 1)).toList with
    | .ok senv' => symEval m senv' fuel ty lastId
    | .error o => o

/-- Try to construct a user/prelude SUM value from `(C arg…)` / `((. T C) arg…)`. `some outcome` if the
head resolves to a DECLARED constructor (MIRRORING `evalVariantCtor`'s erasure so the symbolic value matches
the concrete one); `none` if it is not a ctor (fall through to the operator/call dispatch), or if a local
binding shadows the name. Erasure (identical to the concrete evaluator, via the same helpers): a NEWTYPE
ctor → its payload (no tag); a STRUCT-NEWTYPE → the bare tuple of fields; a SOLE-NULLARY ctor → `unit`; any
other declared ctor → a tagged `.ctor` (arity 1 → single payload; arity ≥2 → a tuple payload). An
unmodelable arg or an arity mismatch (partial application) → `cannotProve`. -/
partial def symCtorConstruct (m : Module) (senv : SymEnv) (fuel : Nat) (ty : IntTy) (children : Array Nat) : Option SymOutcome :=
  match ctorAppName? m children with
  | none => none
  | some cname =>
    if (senv.find? (fun p => p.1 == cname)).isSome then none
    -- newtype/struct-newtype/sole-nullary are checked INDEPENDENTLY of `variantCtorArity?` (which returns
    -- `none` for an erasing ctor) — mirror `evalNode`'s ctorConstruct order. If none of these AND
    -- `variantCtorArity?` is `none`, the head is NOT a declared ctor → fall through (`none`).
    else if !(newtypeCtor? m cname || structNewtypeCtor? m cname || soleNullaryCtor? m cname || (variantCtorArity? m cname).isSome) then none
    -- NULLARY ctors dispatch on the DECLARED arity and IGNORE any actual args — `evalVariantCtor` returns
    -- `.unit` / `.variant cname .unit` for arity 0 regardless of `children` (Eval.lean:1830-1834; the args
    -- are evaluated only to poison and DISCARDED, so even a trapping arg is inert). The cadenza backend
    -- lowers every nullary ctor to `(Ctor unit)` — a unit PLACEHOLDER payload, not a real field — so we must
    -- NOT fold that arg. Handle nullary here, BEFORE the arg-fold, so `(Blue unit)` (round-trip form) and
    -- bare `Blue` (atom path) both give the canonical nullary value. SOLE-nullary erases to `unit`.
    else if soleNullaryCtor? m cname then some (SymOutcome.sym (.const .unit))
    else if variantCtorArity? m cname == some 0 then some (SymOutcome.sym (.ctor cname #[]))
    else
      let argsOpt := (children.extract 1 children.size).foldl (fun (acc : Option (Array SymExpr)) aid =>
        match acc with
        | none => none
        | some arr => (match symEval m senv fuel ty aid with | .sym e => some (arr.push e) | .cannotProve _ => none)) (some #[])
      some (match argsOpt with
        | none => .cannotProve "symeval: user-ctor argument is unmodelable"
        | some args =>
          if newtypeCtor? m cname then (match args[0]? with | some e => .sym e | none => .cannotProve "symeval: newtype ctor missing payload")
          else if structNewtypeCtor? m cname then .sym (.tuple args)
          else match variantCtorArity? m cname with
            | some ar =>
              if args.size != ar then .cannotProve "symeval: constructor arity mismatch (partial application?)"
              else if ar == 1 then .sym (.ctor cname args)
              else .sym (.ctor cname #[.tuple args])
            | none => .cannotProve "symeval: erasing-ctor arity resolution failed")
end

/-- The equivalence VERDICT. `cannotProve` carries a reason: `"boundary"` (hit the incompleteness limit —
weak lead) vs `"normalized-but-different"` (both sides fully normalized yet differ — a STRONG suspected
cadenza-backend miscompile, worth escalating to the sampled differential for confirmation). -/
inductive EquivVerdict where
  | proven
  | cannotProve (reason : String)
  deriving BEq, Inhabited

/-- Symbolic-equivalence of two symbolic outcomes (the input program's and its cadenza round-trip's,
each evaluated with the SAME symbolic input variables). PROVEN iff both normalize to the identical form
(a true forall-inputs statement over the analyzable fragment); otherwise `cannotProve`, distinguishing a
boundary gap from a normalized-but-different strong lead. NEVER a false-divergence claim. -/
def symEquiv (a b : SymOutcome) : EquivVerdict :=
  match a, b with
  | .sym ea, .sym eb => if normalize ea == normalize eb then .proven else .cannotProve "normalized-but-different"
  | .cannotProve r, _ => .cannotProve s!"boundary: {r}"
  | _, .cannotProve r => .cannotProve s!"boundary: {r}"

/-- Symbolically evaluate a program's `exportName` export: bind each parameter to a fresh symbolic input
variable (`var i`, i = its position in the signature), then `symEval` the body. This is the entry the
equivalence check runs on BOTH the input program and its cadenza round-trip, with the SAME var numbering
on each side so a shared input threads through identically. -/
def symEvalExport (m : Module) (exportName : ByteArray) : SymOutcome :=
  match Eval.namedParamsBody? m exportName with
  | some (specs, bodyId) =>
    let senv : SymEnv := (specs.toList.zip (List.range specs.size)).filterMap (fun (specId, idx) =>
      (Eval.paramSpec? m specId).map (fun (nm, _) => (nm, SymExpr.var idx)))
    if senv.length == specs.size then symEval m senv symDefaultFuel defaultIntTy bodyId
    else .cannotProve "symeval: a parameter spec is malformed"
  | none => .cannotProve "symeval: program has no (def (<export> …) BODY)"

/-- Symbolically evaluate the default `main` export. -/
def symEvalMain (m : Module) : SymOutcome := symEvalExport m "main".toUTF8

/-- Prove the input program `mP` and its cadenza round-trip `mP'` functionally equivalent FOR ALL INPUTS
by symbolically evaluating the named export in each (same symbolic input vars) and comparing normal forms.
`proven` = a universal (all-inputs) guarantee; `cannotProve` distinguishes an incompleteness-boundary gap
from a normalized-but-different strong miscompile lead. NEVER a false-divergence claim. -/
def equivExport (mP mP' : Module) (exportName : ByteArray) : EquivVerdict :=
  symEquiv (symEvalExport mP exportName) (symEvalExport mP' exportName)

/-- `equivExport` on the default `main`. -/
def equivMain (mP mP' : Module) : EquivVerdict := equivExport mP mP' "main".toUTF8

-- ── self-tests (the normalize/equiv pipeline; module-level symEval-of-a-real-program1.ast is T2.0b) ──
-- `if true then a else b` normalizes to `a`.
#guard normalize (.ite (.const (.bool true)) (.var 0) (.var 1)) == SymExpr.var 0
-- `if false then a else b` normalizes to `b`.
#guard normalize (.ite (.const (.bool false)) (.var 0) (.var 1)) == SymExpr.var 1
-- `if c then a else a` with a TRAP-FREE condition collapses to `a`.
#guard normalize (.ite (.var 2) (.var 0) (.var 0)) == SymExpr.var 0
-- SOUNDNESS: `if <trapping-cond> then a else a` is NOT collapsed (dropping the trapping condition would be
-- a false "proven" — the original traps, `a` does not). The condition survives in the normal form.
#guard normalize (.ite (.app "/" #[.var 1, .const (.int 0)]) (.var 0) (.var 0))
       == SymExpr.ite (.app "/" #[.var 1, .const (.int 0)]) (.var 0) (.var 0)
-- SOUNDNESS: `if c then +0.0 else -0.0` is NOT collapsed — the derived `==` uses IEEE float equality
-- (`+0.0 == -0.0` is `true`) but +0.0 / -0.0 are OBSERVABLY distinct (`1.0/+0.0 = +inf` vs `-inf`), so
-- collapsing would be a false `proven`. The `symFloatFree` gate keeps the float branches un-collapsed.
#guard normalize (.ite (.var 0) (.const (.f64 (0.0 : Float))) (.const (.f64 (-0.0 : Float))))
       == SymExpr.ite (.var 0) (.const (.f64 (0.0 : Float))) (.const (.f64 (-0.0 : Float)))
-- a float-free `if c then a else a` still collapses (the derived `==` is faithful with no float leaves).
#guard normalize (.ite (.var 5) (.app "+" #[.var 0, .var 1]) (.app "+" #[.var 0, .var 1]))
       == SymExpr.app "+" #[.var 0, .var 1]
-- BOOLEAN MATERIALIZATION: `if c then true else false → c`; `if c then false else true → not c`.
-- A non-const symbolic condition is preserved (operand-preserving; no guard). Even a TRAPPING condition
-- is fine to keep here (it is not dropped — c survives as the result / under the `not`).
#guard normalize (.ite (.var 2) (.const (.bool true)) (.const (.bool false))) == SymExpr.var 2
#guard normalize (.ite (.var 2) (.const (.bool false)) (.const (.bool true))) == SymExpr.app "not" #[.var 2]
#guard normalize (.ite (.app "/" #[.var 1, .const (.int 0)]) (.const (.bool true)) (.const (.bool false)))
       == SymExpr.app "/" #[.var 1, .const (.int 0)]
-- NOT materialized when a branch is not the matching bool literal (stays an ite).
#guard normalize (.ite (.var 2) (.const (.bool true)) (.var 3))
       == SymExpr.ite (.var 2) (.const (.bool true)) (.var 3)
-- T2.0c SOUND constant folding of comparison/boolean ops:
#guard normalize (.app "<" #[.const (.int 2), .const (.int 5)]) == SymExpr.const (.bool true)
#guard normalize (.app ">=" #[.const (.int 2), .const (.int 5)]) == SymExpr.const (.bool false)
#guard normalize (.app "=" #[.const (.int 3), .const (.int 3)]) == SymExpr.const (.bool true)
#guard normalize (.app "and" #[.const (.bool true), .const (.bool false)]) == SymExpr.const (.bool false)
#guard normalize (.app "or" #[.const (.bool false), .const (.bool true)]) == SymExpr.const (.bool true)
#guard normalize (.app "not" #[.const (.bool true)]) == SymExpr.const (.bool false)
-- STRING/Bool/Char/Bytes ordering (value-comparable shapes, via `compareVals`+`cmpHolds` — v-cdz-smith
-- flagged string comparison as a new value-comparable shape; byte-faithful to `evalCmp`). Lexicographic
-- over unsigned bytes: "ab" < "ac"; a proper prefix compares less: "ab" < "abc"; bool false < true.
#guard normalize (.app "<" #[.const (.str "ab".toUTF8), .const (.str "ac".toUTF8)]) == SymExpr.const (.bool true)
#guard normalize (.app ">" #[.const (.str "ab".toUTF8), .const (.str "ac".toUTF8)]) == SymExpr.const (.bool false)
#guard normalize (.app "<" #[.const (.str "ab".toUTF8), .const (.str "abc".toUTF8)]) == SymExpr.const (.bool true)
#guard normalize (.app ">=" #[.const (.str "abc".toUTF8), .const (.str "abc".toUTF8)]) == SymExpr.const (.bool true)
#guard normalize (.app "<" #[.const (.bool false), .const (.bool true)]) == SymExpr.const (.bool true)
-- KEY: a folded comparison CONDITION composes with `if`-selection → proves an optimizer's branch-elim.
#guard normalize (.ite (.app "<" #[.const (.int 1), .const (.int 2)]) (.var 0) (.var 1)) == SymExpr.var 0
-- a comparison with a NON-constant operand is left symbolic (not folded).
#guard normalize (.app "<" #[.var 0, .const (.int 5)]) == SymExpr.app "<" #[.var 0, .const (.int 5)]
-- SOUNDNESS: INTEGER const arithmetic is NOT folded (needs width/overflow-trap semantics) — left symbolic.
#guard normalize (.app "+" #[.const (.int 2), .const (.int 3)]) == SymExpr.app "+" #[.const (.int 2), .const (.int 3)]
-- SOUND integer algebraic identities: x+0→x, x-0→x, x*1→x (operand preserved, never overflows).
#guard normalize (.app "+" #[.var 0, .const (.int 0)]) == SymExpr.var 0
#guard normalize (.app "-" #[.var 0, .const (.int 0)]) == SymExpr.var 0
-- SOUNDNESS: `x - x` does NOT fold to 0 (removed) — `normalize` is type-erased, and for a FLOAT `x`,
-- `x - x` is `.f64 (x-x)` (NaN for x=NaN/inf; not `.int 0` even when finite), so folding would be a false
-- `proven`. It stays symbolic (`x - 0 → x` is kept: the `int 0` literal forces an int context).
#guard normalize (.app "-" #[.var 0, .var 0]) == SymExpr.app "-" #[.var 0, .var 0]
#guard normalize (.app "*" #[.var 0, .const (.int 1)]) == SymExpr.var 0
-- x*0→0 ONLY when the operand is trap-free (a var here); a trapping operand keeps the multiply.
#guard normalize (.app "*" #[.var 0, .const (.int 0)]) == SymExpr.const (.int 0)
#guard normalize (.app "*" #[.app "/" #[.var 0, .const (.int 0)], .const (.int 0)])
       == SymExpr.app "*" #[.app "/" #[.var 0, .const (.int 0)], .const (.int 0)]
-- float x+0.0 is NOT simplified (unsound: -0.0) — stays symbolic.
#guard normalize (.app "+" #[.var 0, .const (.f64 0.0)]) == SymExpr.app "+" #[.var 0, .const (.f64 0.0)]
-- BITWISE ops (now modeled symbolically by symEval, #widening): kept symbolic (foldConst? does not fold
-- bitwise), so P and its round-trip compare structurally; a bitwise op is trap-classified so a `case`/`if`
-- over it is NOT dropped. `&`/`|`/`^`/shifts over vars or consts stay as their `app`.
#guard normalize (.app "&" #[.var 0, .var 1]) == SymExpr.app "&" #[.var 0, .var 1]
#guard normalize (.app "<<" #[.var 0, .const (.int 3)]) == SymExpr.app "<<" #[.var 0, .const (.int 3)]
#guard normalize (.app "|" #[.const (.int 12), .const (.int 10)]) == SymExpr.app "|" #[.const (.int 12), .const (.int 10)]
-- SOUNDNESS: a bitwise op MAY trap (shift-out-of-range), so an equal-branch `if` over it does NOT collapse.
#guard normalize (.ite (.app "<<" #[.var 0, .var 1]) (.var 2) (.var 2))
       == SymExpr.ite (.app "<<" #[.var 0, .var 1]) (.var 2) (.var 2)
-- SOUND BITWISE algebraic identities (width-independent): x|0→x, x&0→0 (guarded), x^0→x, x<<0/x>>0→x, x&x/x|x→x.
#guard normalize (.app "|" #[.var 0, .const (.int 0)]) == SymExpr.var 0
#guard normalize (.app "^" #[.var 0, .const (.int 0)]) == SymExpr.var 0
#guard normalize (.app "<<" #[.var 0, .const (.int 0)]) == SymExpr.var 0
#guard normalize (.app "&" #[.var 0, .const (.int 0)]) == SymExpr.const (.int 0)
#guard normalize (.app "&" #[.var 0, .var 0]) == SymExpr.var 0
#guard normalize (.app "|" #[.var 3, .var 3]) == SymExpr.var 3
-- x^x→0 (XOR zeroing idiom) ONLY when the dropped operand is trap-free; a trapping operand keeps the `^`.
#guard normalize (.app "^" #[.var 0, .var 0]) == SymExpr.const (.int 0)
#guard normalize (.app "^" #[.app "/" #[.var 0, .const (.int 0)], .app "/" #[.var 0, .const (.int 0)]])
       == SymExpr.app "^" #[.app "/" #[.var 0, .const (.int 0)], .app "/" #[.var 0, .const (.int 0)]]
-- x&0→0 ONLY when the dropped operand is trap-free; a trapping operand keeps the `&` (trap preserved).
#guard normalize (.app "&" #[.app "/" #[.var 0, .const (.int 0)], .const (.int 0)])
       == SymExpr.app "&" #[.app "/" #[.var 0, .const (.int 0)], .const (.int 0)]
-- SHORT-CIRCUIT boolean identities: (or true X)→true, (or X false)→X, (and false X)→false, (and X true)→X.
#guard normalize (.app "or" #[.const (.bool true), .var 0]) == SymExpr.const (.bool true)
#guard normalize (.app "or" #[.var 0, .const (.bool false)]) == SymExpr.var 0
#guard normalize (.app "and" #[.const (.bool false), .var 0]) == SymExpr.const (.bool false)
#guard normalize (.app "and" #[.var 0, .const (.bool true)]) == SymExpr.var 0
-- (or X true)→true only when X is trap-free (a var here); a trapping X keeps the or (X is evaluated first).
#guard normalize (.app "or" #[.var 0, .const (.bool true)]) == SymExpr.const (.bool true)
#guard normalize (.app "or" #[.app "/" #[.var 0, .const (.int 0)], .const (.bool true)])
       == SymExpr.app "or" #[.app "/" #[.var 0, .const (.int 0)], .const (.bool true)]
-- BOOLEAN IDEMPOTENCE (x and x → x, x or x → x — bool companions of x&x→x/x|x→x) and DOUBLE-NEGATION
-- (not (not x) → x, bool involution). Operand-preserving (no !mayTrap guard); a single `not` is untouched.
#guard normalize (.app "and" #[.var 0, .var 0]) == SymExpr.var 0
#guard normalize (.app "or" #[.var 5, .var 5]) == SymExpr.var 5
#guard normalize (.app "not" #[.app "not" #[.var 0]]) == SymExpr.var 0
#guard normalize (.app "not" #[.var 0]) == SymExpr.app "not" #[.var 0]
#guard normalize (.app "not" #[.app "not" #[.app "not" #[.var 0]]]) == SymExpr.app "not" #[.var 0]
-- FLOAT arithmetic IS folded (IEEE total, no trap → sound): mirrors v-cdz-smith's fp-1/fp-2 false-positives.
#guard normalize (.app "+" #[.const (.f64 475.0), .const (.f64 514.0)]) == SymExpr.const (.f64 989.0)
#guard normalize (.app "*" #[.const (.f64 6.0), .const (.f64 7.0)]) == SymExpr.const (.f64 42.0)
-- FLOAT comparison folds (mirrors fp-0 `(< 681.0 302.0)` → false).
#guard normalize (.app "<" #[.const (.f64 681.0), .const (.f64 302.0)]) == SymExpr.const (.bool false)
-- SOUNDNESS regression guards for the float `<` fold (IEEE-faithful, matching the `<` operator): `NaN < x`
-- is false, and `+0.0 < -0.0` is false (they compare equal under `<`). (`asF64?` returns `none` on an
-- int, so int `<` is NOT float-folded — that stays the `.int`-pattern arm; audited alongside #6533/#6534.)
#guard normalize (.app "<" #[.const (.f64 (0.0 / 0.0 : Float)), .const (.f64 (1.0 : Float))]) == SymExpr.const (.bool false)
#guard normalize (.app "<" #[.const (.f64 (0.0 : Float)), .const (.f64 (-0.0 : Float))]) == SymExpr.const (.bool false)
-- SOUNDNESS regression guards for the `=` fold's float handling (`valueEqSpec`→`specFloatEq`, BIT-faithful
-- with a canonical NaN — NOT the derived IEEE `==` that made the ite-collapse unsound, #6533). Distinct
-- bits ⇒ NOT equal even when IEEE `==` says equal (`+0.0`/`-0.0`); a canonical NaN ⇒ `NaN = NaN` is true.
#guard normalize (.app "=" #[.const (.f64 (0.0 : Float)), .const (.f64 (-0.0 : Float))]) == SymExpr.const (.bool false)
#guard normalize (.app "=" #[.const (.f64 (1.5 : Float)), .const (.f64 (1.5 : Float))]) == SymExpr.const (.bool true)
#guard normalize (.app "=" #[.const (.f64 (0.0 / 0.0 : Float)), .const (.f64 (0.0 / 0.0 : Float))]) == SymExpr.const (.bool true)
-- a nested float-arith chain folds so it matches the backend's folded literal (fp-1's (+ (+ 475 514) 718)=1707).
#guard symEquiv (.sym (.app "+" #[.app "+" #[.const (.f64 475.0), .const (.f64 514.0)], .const (.f64 718.0)]))
                (.sym (.const (.f64 1707.0))) == EquivVerdict.proven
-- two structurally-identical symbolic forms are PROVEN equivalent for all inputs.
#guard symEquiv (.sym (.app "+" #[.var 0, .const (.int 1)])) (.sym (.app "+" #[.var 0, .const (.int 1)])) == EquivVerdict.proven
-- an optimizer that turned `if true then (x+1) else y` into `x+1` is PROVEN equivalent (const-cond select).
#guard symEquiv (.sym (.ite (.const (.bool true)) (.app "+" #[.var 0, .const (.int 1)]) (.var 1)))
                (.sym (.app "+" #[.var 0, .const (.int 1)])) == EquivVerdict.proven
-- genuinely different symbolic forms → cannotProve (never a false "proven").
#guard symEquiv (.sym (.var 0)) (.sym (.var 1)) == EquivVerdict.cannotProve "normalized-but-different"
-- an unmodeled operand poisons the whole side → boundary cannotProve.
#guard symEquiv (.cannotProve "unmodeled") (.sym (.var 0)) == EquivVerdict.cannotProve "boundary: unmodeled"
-- `=` folds over COMPOUND constants (tuples of consts), not just scalars — closes v-cdz-smith's short-circuit
-- boolean FP whose root was `(= (tuple …) (tuple …))` staying symbolic. Equal tuples → true, differing → false.
#guard normalize (.app "=" #[.tuple #[.const (.int 1), .const (.int 2)], .tuple #[.const (.int 1), .const (.int 2)]]) == SymExpr.const (.bool true)
#guard normalize (.app "=" #[.tuple #[.const (.int 1), .const (.int 2)], .tuple #[.const (.int 1), .const (.int 3)]]) == SymExpr.const (.bool false)
-- and then `(or false true)` / `(or true …)` fold to true via the existing scalar bool fold → the cascade closes.
#guard normalize (.app "or" #[.app "=" #[.tuple #[.const (.int 5)], .tuple #[.const (.int 6)]], .const (.bool true)]) == SymExpr.const (.bool true)

-- ── T2.0b: symEval over a real program Module (bind params → vars) ──
-- `(do (def (main) N) (export main))` — a nullary main whose body is the literal N (the proven program
-- shape from Batch's `_batchHolds`). symEvalMain evaluates the body to `const N`.
private def _progMain (n : UInt8) : Module :=
  { leaves := #[Leaf.name "do".toUTF8, Leaf.name "def".toUTF8, Leaf.name "main".toUTF8,
                Leaf.intLit false .dec (ByteArray.mk #[n]), Leaf.name "export".toUTF8],
    nodes := #[.atom 1, .atom 2, .list #[1], .atom 3, .list #[0, 2, 3],
               .atom 4, .atom 2, .list #[5, 6], .atom 0, .list #[8, 4, 7]],
    root := 9 }
-- symEvalMain reaches the body literal → `const 42`.
#guard symEvalMain (_progMain 42) == SymOutcome.sym (.const (.int 42))
-- a program is PROVEN equivalent to itself for all inputs.
#guard equivMain (_progMain 42) (_progMain 42) == EquivVerdict.proven
-- two programs with different constant bodies are NOT proven equivalent (never a false "proven").
#guard equivMain (_progMain 42) (_progMain 43) == EquivVerdict.cannotProve "normalized-but-different"

-- `let`: `(let ((x 5)) x)` symbolically evaluates to `const 5` (sequential bind then body-lookup).
-- leaves 0:let 1:x 2:(5); nodes 0:.atom x, 1:.atom 5, 2:(x 5) pair, 3:((x 5)) bindings, 4:.atom x body, 5:.atom let, 6:(let …).
private def _letExpr : Module :=
  { leaves := #[Leaf.name "let".toUTF8, Leaf.name "x".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[5])],
    nodes := #[.atom 1, .atom 2, .list #[0, 1], .list #[2], .atom 1, .atom 0, .list #[5, 3, 4]],
    root := 6 }
#guard symEval _letExpr [] symDefaultFuel defaultIntTy 6 == SymOutcome.sym (.const (.int 5))

-- tuples + positional projection: `(. (tuple 7 8) 1)`.
-- leaves 0:tuple 1:(7) 2:(8) 3:. 4:(1); nodes 0-2 atoms, 3:(tuple 7 8), 4:.atom `.`, 5:.atom idx, 6:(. (tuple 7 8) 1).
private def _projExpr : Module :=
  { leaves := #[Leaf.name "tuple".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[7]),
                Leaf.intLit false .dec (ByteArray.mk #[8]), Leaf.name ".".toUTF8,
                Leaf.intLit false .dec (ByteArray.mk #[1])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .list #[4, 3, 5]],
    root := 6 }
-- constructing `(tuple 7 8)` → `.tuple [const 7, const 8]` (node 3).
#guard symEval _projExpr [] symDefaultFuel defaultIntTy 3 == SymOutcome.sym (.tuple #[.const (.int 7), .const (.int 8)])
-- projecting element 1 of `(tuple 7 8)` → `const 8`.
#guard symEval _projExpr [] symDefaultFuel defaultIntTy 6 == SymOutcome.sym (.const (.int 8))

-- records + field projection: `(. (record (a 1) (b 2)) b)`. leaves 0:record 1:a 2:(1) 3:b 4:(2) 5:.
-- nodes: 2:(a 1), 5:(b 2), 7:(record …), 10:(. (record …) b) [field leaf 3 reused].
private def _recExpr : Module :=
  { leaves := #[Leaf.name "record".toUTF8, Leaf.name "a".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.name "b".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[2]), Leaf.name ".".toUTF8],
    nodes := #[.atom 1, .atom 2, .list #[0, 1], .atom 3, .atom 4, .list #[3, 4],
               .atom 0, .list #[6, 2, 5], .atom 5, .atom 3, .list #[8, 7, 9]],
    root := 10 }
-- constructing `(record (a 1) (b 2))` → fields sorted by key `[(a,1),(b,2)]` (node 7).
#guard symEval _recExpr [] symDefaultFuel defaultIntTy 7 == SymOutcome.sym (.record #[("a".toUTF8, .const (.int 1)), ("b".toUTF8, .const (.int 2))])
-- projecting field `b` → `const 2`.
#guard symEval _recExpr [] symDefaultFuel defaultIntTy 10 == SymOutcome.sym (.const (.int 2))

-- built-in Option/Result construction: `(Some 5)` → `.ctor "Some" [const 5]`; bare `None` → `.ctor "None" []`.
private def _someExpr : Module :=
  { leaves := #[Leaf.name "Some".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[5])],
    nodes := #[.atom 0, .atom 1, .list #[0, 1]], root := 2 }
#guard symEval _someExpr [] symDefaultFuel defaultIntTy 2 == SymOutcome.sym (.ctor "Some".toUTF8 #[.const (.int 5)])
private def _noneExpr : Module :=
  { leaves := #[Leaf.name "None".toUTF8], nodes := #[.atom 0], root := 0 }
#guard symEval _noneExpr [] symDefaultFuel defaultIntTy 0 == SymOutcome.sym (.ctor "None".toUTF8 #[])

-- non-recursive CALL inlining: `(do (def (id x) x) (def (main) (id 42)) (export main))`. main's body
-- `(id 42)` inlines the top-level `id` (bind x→const 42, eval body `x`) → const 42.
private def _callProg : Module :=
  { leaves := #[Leaf.name "do".toUTF8, Leaf.name "def".toUTF8, Leaf.name "id".toUTF8, Leaf.name "x".toUTF8,
                Leaf.name "main".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[42]), Leaf.name "export".toUTF8],
    nodes := #[.atom 1, .atom 2, .atom 3, .list #[1, 2], .atom 3, .list #[0, 3, 4],
               .atom 1, .atom 4, .list #[7], .atom 2, .atom 5, .list #[9, 10], .list #[6, 8, 11],
               .atom 6, .atom 4, .list #[13, 14], .atom 0, .list #[16, 5, 12, 15]],
    root := 17 }
#guard symEvalMain _callProg == SymOutcome.sym (.const (.int 42))

-- LIST literal coverage: `(list 1 2)` → `.ctor "list" [const 1, const 2]` (ordered; was `cannotProve`
-- before "list" was modeled — the head fell through to call-inlining of a non-existent def).
private def _listExpr : Module :=
  { leaves := #[Leaf.name "list".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[2])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2]], root := 3 }
#guard symEval _listExpr [] symDefaultFuel defaultIntTy 3
       == SymOutcome.sym (.ctor "list".toUTF8 #[.const (.int 1), .const (.int 2)])

-- SET literal coverage: `(set 1 2)` → `.ctor "set" [const 1, const 2]` (canonicalized; already sorted).
private def _setExpr : Module :=
  { leaves := #[Leaf.name "set".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[2])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2]], root := 3 }
#guard symEval _setExpr [] symDefaultFuel defaultIntTy 3
       == SymOutcome.sym (.ctor "set".toUTF8 #[.const (.int 1), .const (.int 2)])

-- SET literal CANONICALIZATION (reorder equality): `(set 2 1)` canonicalizes to the SAME `[1,2]` as
-- `(set 1 2)` — so a set literal and its round-trip reordering now PROVE equal (was a blind spot).
private def _setReorderExpr : Module :=
  { leaves := #[Leaf.name "set".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[2]),
                Leaf.intLit false .dec (ByteArray.mk #[1])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2]], root := 3 }
#guard symEval _setReorderExpr [] symDefaultFuel defaultIntTy 3
       == SymOutcome.sym (.ctor "set".toUTF8 #[.const (.int 1), .const (.int 2)])

-- MAP literal coverage: `(map (1 10) (2 20))` → `.ctor "map" [tuple[1,10], tuple[2,20]]` (canonicalized).
private def _mapExpr : Module :=
  { leaves := #[Leaf.name "map".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[10]), Leaf.intLit false .dec (ByteArray.mk #[2]),
                Leaf.intLit false .dec (ByteArray.mk #[20])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[1, 2], .atom 3, .atom 4, .list #[4, 5], .list #[0, 3, 6]],
    root := 7 }
#guard symEval _mapExpr [] symDefaultFuel defaultIntTy 7
       == SymOutcome.sym (.ctor "map".toUTF8 #[.tuple #[.const (.int 1), .const (.int 10)],
                                               .tuple #[.const (.int 2), .const (.int 20)]])

-- MAP literal CANONICALIZATION (key-reorder equality): `(map (2 20) (1 10))` sorts-by-key to the SAME
-- `[(1,10),(2,20)]` as `(map (1 10) (2 20))`.
private def _mapReorderExpr : Module :=
  { leaves := #[Leaf.name "map".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[2]),
                Leaf.intLit false .dec (ByteArray.mk #[20]), Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[10])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[1, 2], .atom 3, .atom 4, .list #[4, 5], .list #[0, 3, 6]],
    root := 7 }
#guard symEval _mapReorderExpr [] symDefaultFuel defaultIntTy 7
       == SymOutcome.sym (.ctor "map".toUTF8 #[.tuple #[.const (.int 1), .const (.int 10)],
                                               .tuple #[.const (.int 2), .const (.int 20)]])

-- LIST.CONCAT member-op coverage: `((. List concat) (list 1) (list 2))` → `.ctor "list" [const 1, const 2]`
-- (appends the element arrays). Member-calls were previously `cannotProve` ("non-name head").
private def _concatExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "List".toUTF8, Leaf.name "concat".toUTF8,
                Leaf.name "list".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[2])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .list #[4, 5],
               .atom 3, .atom 5, .list #[7, 8], .list #[3, 6, 9]],
    root := 10 }
#guard symEval _concatExpr [] symDefaultFuel defaultIntTy 10
       == SymOutcome.sym (.ctor "list".toUTF8 #[.const (.int 1), .const (.int 2)])

-- LIST.LEN member-op coverage: `((. List len) (list 1 2 3))` → `const 3` (element count).
private def _lenExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "List".toUTF8, Leaf.name "len".toUTF8,
                Leaf.name "list".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[2]), Leaf.intLit false .dec (ByteArray.mk #[3])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .atom 5, .atom 6,
               .list #[4, 5, 6, 7], .list #[3, 8]],
    root := 9 }
#guard symEval _lenExpr [] symDefaultFuel defaultIntTy 9 == SymOutcome.sym (.const (.int 3))

-- LIST.PUSH member-op coverage: `((. List push) (list 1 2) 3)` → `.ctor "list" [const 1, const 2, const 3]`
-- (order-preserving append of one element, mirrors `evalNode`'s `es.push x`).
private def _pushExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "List".toUTF8, Leaf.name "push".toUTF8,
                Leaf.name "list".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[2]), Leaf.intLit false .dec (ByteArray.mk #[3])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .atom 5,
               .list #[4, 5, 6], .atom 6, .list #[3, 7, 8]],
    root := 9 }
#guard symEval _pushExpr [] symDefaultFuel defaultIntTy 9
       == SymOutcome.sym (.ctor "list".toUTF8 #[.const (.int 1), .const (.int 2), .const (.int 3)])

-- LIST.PREPEND coverage (v-cdz-smith #7513): `((. List prepend) (list 2 3) 1)` → `[1,2,3]` (cons at FRONT).
private def _listPrependExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "List".toUTF8, Leaf.name "prepend".toUTF8,
                Leaf.name "list".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[2]),
                Leaf.intLit false .dec (ByteArray.mk #[3]), Leaf.intLit false .dec (ByteArray.mk #[1])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .atom 5,
               .list #[4, 5, 6], .atom 6, .list #[3, 7, 8]],
    root := 9 }
#guard symEval _listPrependExpr [] symDefaultFuel defaultIntTy 9
       == SymOutcome.sym (.ctor "list".toUTF8 #[.const (.int 1), .const (.int 2), .const (.int 3)])

-- MAP.MERGE coverage (v-cdz-smith #7513): `((. Map merge) (map (1 10) (2 20)) (map (2 99) (3 30)))` →
-- `{1:10, 2:99, 3:30}` — the RIGHT operand wins on the shared key 2 (99, not 20). Pins the b-wins order.
private def _mapMergeExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Map".toUTF8, Leaf.name "merge".toUTF8,
                Leaf.name "map".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[10]), Leaf.intLit false .dec (ByteArray.mk #[2]),
                Leaf.intLit false .dec (ByteArray.mk #[20]), Leaf.intLit false .dec (ByteArray.mk #[99]),
                Leaf.intLit false .dec (ByteArray.mk #[3]), Leaf.intLit false .dec (ByteArray.mk #[30])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2],           -- 0..3  (. Map merge)
               .atom 4, .atom 5, .list #[4, 5], .atom 6, .atom 7, .list #[7, 8], .atom 3, .list #[10, 6, 9], -- 4..11 (map (1 10)(2 20))
               .atom 6, .atom 8, .list #[12, 13], .atom 9, .atom 10, .list #[15, 16], .atom 3, .list #[18, 14, 17], -- 12..19 (map (2 99)(3 30))
               .list #[3, 11, 19]],                                   -- 20   ((. Map merge) a b)
    root := 20 }
#guard symEval _mapMergeExpr [] symDefaultFuel defaultIntTy 20
       == SymOutcome.sym (.ctor "map".toUTF8 #[.tuple #[.const (.int 1), .const (.int 10)],
                                               .tuple #[.const (.int 2), .const (.int 99)],
                                               .tuple #[.const (.int 3), .const (.int 30)]])

-- LIST.AT/GET member-op coverage: indexed access → Option. `((. List at) (list 10 20 30) 1)` → `Some 20`
-- (in-bounds); `((. List at) (list 10 20 30) 5)` → `None` (out-of-bounds). Purely structural (no equality).
private def _atInExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "List".toUTF8, Leaf.name "at".toUTF8,
                Leaf.name "list".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[10]),
                Leaf.intLit false .dec (ByteArray.mk #[20]), Leaf.intLit false .dec (ByteArray.mk #[30]),
                Leaf.intLit false .dec (ByteArray.mk #[1])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .atom 5, .atom 6,
               .list #[4, 5, 6, 7], .atom 7, .list #[3, 8, 9]],
    root := 10 }
#guard symEval _atInExpr [] symDefaultFuel defaultIntTy 10
       == SymOutcome.sym (.ctor "Some".toUTF8 #[.const (.int 20)])

private def _atOobExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "List".toUTF8, Leaf.name "at".toUTF8,
                Leaf.name "list".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[10]),
                Leaf.intLit false .dec (ByteArray.mk #[20]), Leaf.intLit false .dec (ByteArray.mk #[30]),
                Leaf.intLit false .dec (ByteArray.mk #[5])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .atom 5, .atom 6,
               .list #[4, 5, 6, 7], .atom 7, .list #[3, 8, 9]],
    root := 10 }
#guard symEval _atOobExpr [] symDefaultFuel defaultIntTy 10
       == SymOutcome.sym (.ctor "None".toUTF8 #[])

-- STRING.CONCAT member-op coverage: `((. String concat) "ab" "cd")` → `.const (.str "abcd")`.
private def _strConcatExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "String".toUTF8, Leaf.name "concat".toUTF8,
                Leaf.str "ab".toUTF8, Leaf.str "cd".toUTF8],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .list #[3, 4, 5]],
    root := 6 }
#guard symEval _strConcatExpr [] symDefaultFuel defaultIntTy 6
       == SymOutcome.sym (.const (.str "abcd".toUTF8))

-- BYTES.CONCAT member-op coverage: `((. Bytes concat) #{1,2} #{3})` → `.const (.bytes #{1,2,3})`.
private def _bytesConcatExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Bytes".toUTF8, Leaf.name "concat".toUTF8,
                Leaf.bytesLit (ByteArray.mk #[1, 2]), Leaf.bytesLit (ByteArray.mk #[3])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .list #[3, 4, 5]],
    root := 6 }
#guard symEval _bytesConcatExpr [] symDefaultFuel defaultIntTy 6
       == SymOutcome.sym (.const (.bytes (ByteArray.mk #[1, 2, 3])))

-- SET.CONTAINS member-op coverage: `((. Set contains) (set 1 2 3) q)` → `.const (.bool (q ∈ {1,2,3}))`
-- (bit-faithful valEq membership over concrete elements). q=2 → true; q=5 → false.
private def _setContainsTrueExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Set".toUTF8, Leaf.name "contains".toUTF8,
                Leaf.name "set".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[2]), Leaf.intLit false .dec (ByteArray.mk #[3]),
                Leaf.intLit false .dec (ByteArray.mk #[2])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .atom 5, .atom 6,
               .list #[4, 5, 6, 7], .atom 7, .list #[3, 8, 9]],
    root := 10 }
#guard symEval _setContainsTrueExpr [] symDefaultFuel defaultIntTy 10
       == SymOutcome.sym (.const (.bool true))

private def _setContainsFalseExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Set".toUTF8, Leaf.name "contains".toUTF8,
                Leaf.name "set".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[2]), Leaf.intLit false .dec (ByteArray.mk #[3]),
                Leaf.intLit false .dec (ByteArray.mk #[5])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .atom 5, .atom 6,
               .list #[4, 5, 6, 7], .atom 7, .list #[3, 8, 9]],
    root := 10 }
#guard symEval _setContainsFalseExpr [] symDefaultFuel defaultIntTy 10
       == SymOutcome.sym (.const (.bool false))

-- STRING.BYTE-LEN / SCALAR-LEN member-op coverage over "café" (é = 2 UTF-8 bytes → byte-len 5, scalar-len 4).
private def _byteLenExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "String".toUTF8, Leaf.name "byte-len".toUTF8,
                Leaf.str "café".toUTF8],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .list #[3, 4]],
    root := 5 }
#guard symEval _byteLenExpr [] symDefaultFuel defaultIntTy 5
       == SymOutcome.sym (.const (.int 5))

private def _scalarLenExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "String".toUTF8, Leaf.name "scalar-len".toUTF8,
                Leaf.str "café".toUTF8],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .list #[3, 4]],
    root := 5 }
#guard symEval _scalarLenExpr [] symDefaultFuel defaultIntTy 5
       == SymOutcome.sym (.const (.int 4))

-- STRING.SLICE member-op coverage: `((. String slice) "hello" 1 4)` → `Some "ell"` (code-points [1,4));
-- `((. String slice) "hello" 2 10)` → `None` (end past scalar-count).
private def _sliceInExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "String".toUTF8, Leaf.name "slice".toUTF8,
                Leaf.str "hello".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[4])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .atom 5, .list #[3, 4, 5, 6]],
    root := 7 }
#guard symEval _sliceInExpr [] symDefaultFuel defaultIntTy 7
       == SymOutcome.sym (.ctor "Some".toUTF8 #[.const (.str "ell".toUTF8)])

private def _sliceOobExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "String".toUTF8, Leaf.name "slice".toUTF8,
                Leaf.str "hello".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[2]),
                Leaf.intLit false .dec (ByteArray.mk #[10])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .atom 5, .list #[3, 4, 5, 6]],
    root := 7 }
#guard symEval _sliceOobExpr [] symDefaultFuel defaultIntTy 7
       == SymOutcome.sym (.ctor "None".toUTF8 #[])

-- STRING.AT member-op coverage: `((. String at) "café" 3)` → `Some "é"` (scalar index 3 = é); OOB → None.
private def _strAtInExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "String".toUTF8, Leaf.name "at".toUTF8,
                Leaf.str "café".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[3])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .list #[3, 4, 5]],
    root := 6 }
#guard symEval _strAtInExpr [] symDefaultFuel defaultIntTy 6
       == SymOutcome.sym (.ctor "Some".toUTF8 #[.const (.str "é".toUTF8)])

private def _strAtOobExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "String".toUTF8, Leaf.name "at".toUTF8,
                Leaf.str "café".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[10])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .list #[3, 4, 5]],
    root := 6 }
#guard symEval _strAtOobExpr [] symDefaultFuel defaultIntTy 6
       == SymOutcome.sym (.ctor "None".toUTF8 #[])

-- STRING.SCALAR-AT: `((. String scalar-at) "hello" 1)` → Some #\e (Option<Char> — the CHAR value).
private def _strScalarAtExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "String".toUTF8, Leaf.name "scalar-at".toUTF8,
                Leaf.str "hello".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .list #[3, 4, 5]],
    root := 6 }
#guard symEval _strScalarAtExpr [] symDefaultFuel defaultIntTy 6
       == SymOutcome.sym (.ctor "Some".toUTF8 #[.const (.char "e".toUTF8)])

-- INT-CONVERSION `.of`/`.wrap` member-op coverage (v-cdz-smith top boundary, ~60 cases). Single-arg call
-- `((. <IntTy> of|wrap) x)`: leaves [".", IntTy, of|wrap, x]; nodes head-list#[0,1,2] + arg-atom + call.
private def _int8OfExpr : Module :=  -- (Int8.of 2) → 2 (in range [-128,128))
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Int8".toUTF8, Leaf.name "of".toUTF8,
                Leaf.intLit false .dec (ByteArray.mk #[2])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .list #[3, 4]], root := 5 }
#guard symEval _int8OfExpr [] symDefaultFuel defaultIntTy 5 == SymOutcome.sym (.const (.int 2))

private def _uint8OfOobExpr : Module :=  -- (UInt8.of 300) → OUT of range [0,256) → traps → cannotProve
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "UInt8".toUTF8, Leaf.name "of".toUTF8,
                Leaf.intLit false .dec (ByteArray.mk #[1, 44])],  -- 1*256 + 44 = 300
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .list #[3, 4]], root := 5 }
#guard match symEval _uint8OfOobExpr [] symDefaultFuel defaultIntTy 5 with
       | .cannotProve _ => true | _ => false

private def _uint8WrapExpr : Module :=  -- (UInt8.wrap 300) → 300 mod 256 = 44 (total)
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "UInt8".toUTF8, Leaf.name "wrap".toUTF8,
                Leaf.intLit false .dec (ByteArray.mk #[1, 44])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .list #[3, 4]], root := 5 }
#guard symEval _uint8WrapExpr [] symDefaultFuel defaultIntTy 5 == SymOutcome.sym (.const (.int 44))

private def _int64OfExpr : Module :=  -- (Int64.of 70) → 70 (in range)
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Int64".toUTF8, Leaf.name "of".toUTF8,
                Leaf.intLit false .dec (ByteArray.mk #[70])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .list #[3, 4]], root := 5 }
#guard symEval _int64OfExpr [] symDefaultFuel defaultIntTy 5 == SymOutcome.sym (.const (.int 70))

-- FLOAT64.NAN prelude-constant member-access `(. Float64 nan)` → the NaN value (v-cdz-smith next tier).
private def _float64NanExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Float64".toUTF8, Leaf.name "nan".toUTF8],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2]], root := 3 }
#guard symEval _float64NanExpr [] symDefaultFuel defaultIntTy 3 == SymOutcome.sym (.const .floatNan)

private def _float64InfExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Float64".toUTF8, Leaf.name "Infinity".toUTF8],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2]], root := 3 }
#guard symEval _float64InfExpr [] symDefaultFuel defaultIntTy 3 == SymOutcome.sym (.const (.floatInf false))

-- Float64.nan in a CALL/member-head position `((. Float64 nan))` also folds (companion to the proj form).
private def _float64NanCallExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Float64".toUTF8, Leaf.name "nan".toUTF8],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .list #[3]], root := 4 }
#guard symEval _float64NanCallExpr [] symDefaultFuel defaultIntTy 4 == SymOutcome.sym (.const .floatNan)
-- and a float ORDERING with a NaN constant operand folds to `false` (IEEE: any NaN comparison is false).
#guard normalize (.app "<=" #[.const .floatNan, .const (.f64 (834.0 : Float))]) == SymExpr.const (.bool false)
#guard normalize (.app ">" #[.const (.floatInf false), .const (.f64 (5.0 : Float))]) == SymExpr.const (.bool true)

-- OPTION.EXPECT member-op coverage: `((. Option expect) (Some 5))` → 5 (unwrap the Some payload).
private def _optExpectExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Option".toUTF8, Leaf.name "expect".toUTF8,
                Leaf.name "Some".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[5])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .list #[4, 5],
               .list #[3, 6]],
    root := 7 }
#guard symEval _optExpectExpr [] symDefaultFuel defaultIntTy 7 == SymOutcome.sym (.const (.int 5))

-- RATIONAL.OF member-op coverage: `((. Rational of) 6 4)` → the normalized rational 3/2 (gcd-reduced).
private def _ratOfExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Rational".toUTF8, Leaf.name "of".toUTF8,
                Leaf.intLit false .dec (ByteArray.mk #[6]), Leaf.intLit false .dec (ByteArray.mk #[4])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .list #[3, 4, 5]],
    root := 6 }
#guard symEval _ratOfExpr [] symDefaultFuel defaultIntTy 6 == SymOutcome.sym (.const (.rational 3 2))
-- RATIONAL.OF zero-numerator sign cross-edge (v-spec-oracle #6797, 06-numeric-model): a ZERO numerator with
-- a NEGATIVE denominator canonicalizes to 0/1 (NOT 0/-1 / signed zero) — intersection of lowest-terms + the
-- sign-on-numerator rule. `((. Rational of) 0 -5)` → .rational 0/1 (mkRational: den<0 → negate both → (0,5),
-- gcd(0,5)=5 → 0/1). Cross-oracle-verified: agrees with v-spec-oracle's corpus pin.
private def _ratZeroNegExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Rational".toUTF8, Leaf.name "of".toUTF8,
                Leaf.intLit false .dec (ByteArray.mk #[0]), Leaf.intLit true .dec (ByteArray.mk #[5])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .list #[3, 4, 5]],
    root := 6 }
#guard symEval _ratZeroNegExpr [] symDefaultFuel defaultIntTy 6 == SymOutcome.sym (.const (.rational 0 1))
-- RATIONAL ARITHMETIC sign cross-edges (v-spec-oracle #6807): mul of two negatives → positive + reduced;
-- subtraction with a negative result → sign on numerator, reduced. symEval folds via eval's rationalArith.
-- (1) `(* (Rational.of -2 3) (Rational.of -3 4))` = 1/2.
private def _ratMulNegExpr : Module :=
  { leaves := #[Leaf.name "*".toUTF8, Leaf.name ".".toUTF8, Leaf.name "Rational".toUTF8, Leaf.name "of".toUTF8,
                Leaf.intLit true .dec (ByteArray.mk #[2]), Leaf.intLit false .dec (ByteArray.mk #[3]),
                Leaf.intLit true .dec (ByteArray.mk #[3]), Leaf.intLit false .dec (ByteArray.mk #[4])],
    nodes := #[.atom 1, .atom 2, .atom 3, .list #[0, 1, 2], .atom 4, .atom 5, .list #[3, 4, 5],
               .atom 6, .atom 7, .list #[3, 7, 8], .atom 0, .list #[10, 6, 9]],
    root := 11 }
#guard symEval _ratMulNegExpr [] symDefaultFuel defaultIntTy 11 == SymOutcome.sym (.const (.rational 1 2))
-- (3) `(- (Rational.of 1 4) (Rational.of 1 2))` = -1/4.
private def _ratSubNegExpr : Module :=
  { leaves := #[Leaf.name "-".toUTF8, Leaf.name ".".toUTF8, Leaf.name "Rational".toUTF8, Leaf.name "of".toUTF8,
                Leaf.intLit false .dec (ByteArray.mk #[1]), Leaf.intLit false .dec (ByteArray.mk #[4]),
                Leaf.intLit false .dec (ByteArray.mk #[2])],
    nodes := #[.atom 1, .atom 2, .atom 3, .list #[0, 1, 2], .atom 4, .atom 5, .list #[3, 4, 5],
               .atom 4, .atom 6, .list #[3, 7, 8], .atom 0, .list #[10, 6, 9]],
    root := 11 }
#guard symEval _ratSubNegExpr [] symDefaultFuel defaultIntTy 11 == SymOutcome.sym (.const (.rational (-1) 4))
-- RATIONAL divide-by-zero-VALUE boundary (v-spec-oracle #6813): dividing by the zero-VALUED rational 0/1
-- (a valid value, distinct from the zero-DENOMINATOR non-value) is a div-by-zero → rationalArith traps →
-- the fold correctly STAYS SYMBOLIC (not a value). In the differential the backend const-proves the zero
-- divisor and rejects CDZ0304 (program declines), so a symbolic verdict here is safe (never a false proven).
#guard symEval { leaves := #[Leaf.name "/".toUTF8, Leaf.name ".".toUTF8, Leaf.name "Rational".toUTF8,
                             Leaf.name "of".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                             Leaf.intLit false .dec (ByteArray.mk #[2]), Leaf.intLit false .dec (ByteArray.mk #[0])],
                 nodes := #[.atom 1, .atom 2, .atom 3, .list #[0, 1, 2], .atom 4, .atom 5, .list #[3, 4, 5],
                            .atom 6, .atom 4, .list #[3, 7, 8], .atom 0, .list #[10, 6, 9]], root := 11 }
                [] symDefaultFuel defaultIntTy 11
       == SymOutcome.sym (.app "/" #[.const (.rational 1 2), .const (.rational 0 1)])

-- BYTES.OF member-op coverage: `((. Bytes of) (list 10 20 30))` → `.const (.bytes #{10,20,30})`.
private def _bytesOfExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Bytes".toUTF8, Leaf.name "of".toUTF8,
                Leaf.name "list".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[10]),
                Leaf.intLit false .dec (ByteArray.mk #[20]), Leaf.intLit false .dec (ByteArray.mk #[30])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .atom 5, .atom 6,
               .list #[4, 5, 6, 7], .list #[3, 8]],
    root := 9 }
#guard symEval _bytesOfExpr [] symDefaultFuel defaultIntTy 9
       == SymOutcome.sym (.const (.bytes (ByteArray.mk #[10, 20, 30])))

-- BYTES len/at/slice member-op coverage over #{10,20,30,40} (byte-indexed; slice is start/LENGTH).
private def _bytesLenExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Bytes".toUTF8, Leaf.name "len".toUTF8,
                Leaf.bytesLit (ByteArray.mk #[10, 20, 30, 40])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .list #[3, 4]],
    root := 5 }
#guard symEval _bytesLenExpr [] symDefaultFuel defaultIntTy 5
       == SymOutcome.sym (.const (.int 4))

private def _bytesAtExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Bytes".toUTF8, Leaf.name "at".toUTF8,
                Leaf.bytesLit (ByteArray.mk #[10, 20, 30, 40]), Leaf.intLit false .dec (ByteArray.mk #[1])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .list #[3, 4, 5]],
    root := 6 }
#guard symEval _bytesAtExpr [] symDefaultFuel defaultIntTy 6
       == SymOutcome.sym (.ctor "Some".toUTF8 #[.const (.int 20)])

private def _bytesSliceExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Bytes".toUTF8, Leaf.name "slice".toUTF8,
                Leaf.bytesLit (ByteArray.mk #[10, 20, 30, 40]), Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[2])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .atom 5, .list #[3, 4, 5, 6]],
    root := 7 }
#guard symEval _bytesSliceExpr [] symDefaultFuel defaultIntTy 7
       == SymOutcome.sym (.ctor "Some".toUTF8 #[.const (.bytes (ByteArray.mk #[20, 30]))])

-- SET.LEN member-op coverage with DEDUP: `((. Set len) (set 1 2 2 3))` → 3 (distinct count via canonSet).
private def _setLenExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Set".toUTF8, Leaf.name "len".toUTF8,
                Leaf.name "set".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[2]), Leaf.intLit false .dec (ByteArray.mk #[3])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .atom 5, .atom 5, .atom 6,
               .list #[4, 5, 6, 7, 8], .list #[3, 9]],
    root := 10 }
#guard symEval _setLenExpr [] symDefaultFuel defaultIntTy 10
       == SymOutcome.sym (.const (.int 3))

-- SET.LEN over COMPOUND elements: `((. Set len) (set (list 1 2) (list 29 22)))` → 2 (two distinct lists;
-- `symElemToValue?` reifies each list ctor → `Value.list`, `canonSet` counts distinct). v-cdz-smith #3.
private def _setLenCompoundExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Set".toUTF8, Leaf.name "len".toUTF8,
                Leaf.name "set".toUTF8, Leaf.name "list".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[2]), Leaf.intLit false .dec (ByteArray.mk #[29]),
                Leaf.intLit false .dec (ByteArray.mk #[22])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 4, .atom 5, .atom 6, .list #[4, 5, 6],
               .atom 4, .atom 7, .atom 8, .list #[8, 9, 10], .atom 3, .list #[12, 7, 11], .list #[3, 13]],
    root := 14 }
#guard symEval _setLenCompoundExpr [] symDefaultFuel defaultIntTy 14
       == SymOutcome.sym (.const (.int 2))

-- SET.CONTAINS over COMPOUND elements + compound query: `((. Set contains) (set (list 1 2)(list 3 4)) (list 1 2))`
-- → true (`symElemToValue?` reifies each list; `valEq` membership over the Values). No set rebuild.
private def _setContainsCompoundExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Set".toUTF8, Leaf.name "contains".toUTF8,
                Leaf.name "set".toUTF8, Leaf.name "list".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[2]), Leaf.intLit false .dec (ByteArray.mk #[3]),
                Leaf.intLit false .dec (ByteArray.mk #[4])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 4, .atom 5, .atom 6, .list #[4, 5, 6],
               .atom 4, .atom 7, .atom 8, .list #[8, 9, 10], .atom 3, .list #[12, 7, 11],
               .atom 4, .atom 5, .atom 6, .list #[14, 15, 16], .list #[3, 13, 17]],
    root := 18 }
#guard symEval _setContainsCompoundExpr [] symDefaultFuel defaultIntTy 18
       == SymOutcome.sym (.const (.bool true))

-- SET.INSERT over COMPOUND elements: `((. Set insert) (set (list 1 2)) (list 3 4))` → canonical set
-- `{[1,2], [3,4]}` (`symElemToValue?` reifies, `canonSet` sorts, `valueToSym` rebuilds representation-faithfully).
private def _setInsertCompoundExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Set".toUTF8, Leaf.name "insert".toUTF8,
                Leaf.name "set".toUTF8, Leaf.name "list".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[2]), Leaf.intLit false .dec (ByteArray.mk #[3]),
                Leaf.intLit false .dec (ByteArray.mk #[4])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 4, .atom 5, .atom 6, .list #[4, 5, 6],
               .atom 3, .list #[8, 7], .atom 4, .atom 7, .atom 8, .list #[10, 11, 12], .list #[3, 9, 13]],
    root := 14 }
#guard symEval _setInsertCompoundExpr [] symDefaultFuel defaultIntTy 14
       == SymOutcome.sym (.ctor "set".toUTF8 #[.ctor "list".toUTF8 #[.const (.int 1), .const (.int 2)],
                                              .ctor "list".toUTF8 #[.const (.int 3), .const (.int 4)]])

-- SET LITERAL with COMPOUND elements now CANONICALIZES (reorder equality): `(set (list 3 4)(list 1 2))`
-- canonicalizes to the SAME `{[1,2],[3,4]}` as `(set (list 1 2)(list 3 4))` — previously kept source order.
private def _setReorderCompoundExpr : Module :=
  { leaves := #[Leaf.name "set".toUTF8, Leaf.name "list".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[3]),
                Leaf.intLit false .dec (ByteArray.mk #[4]), Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[2])],
    nodes := #[.atom 1, .atom 2, .atom 3, .list #[0, 1, 2], .atom 1, .atom 4, .atom 5, .list #[4, 5, 6],
               .atom 0, .list #[8, 3, 7]],
    root := 9 }
#guard symEval _setReorderCompoundExpr [] symDefaultFuel defaultIntTy 9
       == SymOutcome.sym (.ctor "set".toUTF8 #[.ctor "list".toUTF8 #[.const (.int 1), .const (.int 2)],
                                              .ctor "list".toUTF8 #[.const (.int 3), .const (.int 4)]])

-- QTY (v-cdz-smith #7282 widening): `(Qty.value (Qty.of 5 (Unit.base #"g")))` → 5 (Qty.of builds the
-- (magnitude, unit) `.ctor "qty"`; Qty.value extracts the magnitude; Unit.base builds the unit).
private def _qtyDirectExpr : Module :=
  { leaves := #[.name ".".toUTF8, .name "Qty".toUTF8, .name "value".toUTF8, .name "of".toUTF8,
                .name "Unit".toUTF8, .name "base".toUTF8, .intLit false .dec (ByteArray.mk #[5]),
                .sym "g".toUTF8],
    nodes := #[.atom 0, .atom 4, .atom 5, .list #[0, 1, 2], .atom 7, .list #[3, 4],
               .atom 0, .atom 1, .atom 3, .list #[6, 7, 8], .atom 6, .list #[9, 10, 5],
               .atom 0, .atom 1, .atom 2, .list #[12, 13, 14], .list #[15, 11]],
    root := 16 }
#guard symEval _qtyDirectExpr [] symDefaultFuel defaultIntTy 16 == SymOutcome.sym (.const (.int 5))

-- QTY same-unit ADD: `(Qty.value (+ (Qty.of 6 (Unit.base #"s")) (Qty.of 3 (Unit.base #"s"))))` → 9
-- (the arith path folds two `.ctor "qty"` operands → magnitude 6+3=9; Qty.value extracts it).
private def _qtyAddExpr : Module :=
  { leaves := #[.name ".".toUTF8, .name "Qty".toUTF8, .name "value".toUTF8, .name "of".toUTF8,
                .name "Unit".toUTF8, .name "base".toUTF8, .name "+".toUTF8,
                .intLit false .dec (ByteArray.mk #[6]), .intLit false .dec (ByteArray.mk #[3]),
                .sym "s".toUTF8],
    nodes := #[.atom 0, .atom 4, .atom 5, .list #[0,1,2], .atom 9, .list #[3,4],
               .atom 0, .atom 1, .atom 3, .list #[6,7,8], .atom 7, .list #[9,10,5],
               .atom 0, .atom 4, .atom 5, .list #[12,13,14], .atom 9, .list #[15,16],
               .atom 0, .atom 1, .atom 3, .list #[18,19,20], .atom 8, .list #[21,22,17],
               .atom 6, .list #[24,11,23],
               .atom 0, .atom 1, .atom 2, .list #[26,27,28], .list #[29,25]],
    root := 30 }
#guard symEval _qtyAddExpr [] symDefaultFuel defaultIntTy 30 == SymOutcome.sym (.const (.int 9))

-- QTY with a GROUPED magnitude: `(Qty.value (Qty.of (0) (Unit.base #"s")))` → 0. The `(0)` is a nullary
-- application (grouping) that symEval reduces to `0` (identity), so the Qty folds. (v-cdz-smith's #7227
-- regression-guard grouped magnitudes hit the non-name-head grouping path.)
private def _qtyGroupedExpr : Module :=
  { leaves := #[.name ".".toUTF8, .name "Qty".toUTF8, .name "value".toUTF8, .name "of".toUTF8,
                .name "Unit".toUTF8, .name "base".toUTF8, .intLit false .dec (ByteArray.mk #[0]),
                .sym "s".toUTF8],
    nodes := #[.atom 0, .atom 4, .atom 5, .list #[0, 1, 2], .atom 7, .list #[3, 4],
               .atom 0, .atom 1, .atom 3, .list #[6, 7, 8], .atom 6, .list #[10], .list #[9, 11, 5],
               .atom 0, .atom 1, .atom 2, .list #[13, 14, 15], .list #[16, 12]],
    root := 17 }
#guard symEval _qtyGroupedExpr [] symDefaultFuel defaultIntTy 17 == SymOutcome.sym (.const (.int 0))

-- MAP.LEN member-op coverage with DUP-KEY DEDUP: `((. Map len) (map (1 10) (1 20) (2 30)))` → 2 (key 1
-- deduped, last-wins via canonMap).
private def _mapLenExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Map".toUTF8, Leaf.name "len".toUTF8,
                Leaf.name "map".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[10]), Leaf.intLit false .dec (ByteArray.mk #[20]),
                Leaf.intLit false .dec (ByteArray.mk #[2]), Leaf.intLit false .dec (ByteArray.mk #[30])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 4, .atom 5, .list #[4, 5],
               .atom 4, .atom 6, .list #[7, 8], .atom 7, .atom 8, .list #[10, 11],
               .atom 3, .list #[13, 6, 9, 12], .list #[3, 14]],
    root := 15 }
#guard symEval _mapLenExpr [] symDefaultFuel defaultIntTy 15
       == SymOutcome.sym (.const (.int 2))

-- MAP.LOOKUP member-op coverage. DUP-KEY LAST-WINS: `((. Map lookup) (map (1 10) (1 99)) 1)` → Some 99
-- (canonMap dedups key 1 keeping the LAST value). NOT-FOUND: `((. Map lookup) (map (1 10)) 5)` → None.
private def _mapLookupDupExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Map".toUTF8, Leaf.name "lookup".toUTF8,
                Leaf.name "map".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[10]), Leaf.intLit false .dec (ByteArray.mk #[99])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 4, .atom 5, .list #[4, 5],
               .atom 4, .atom 6, .list #[7, 8], .atom 3, .list #[10, 6, 9], .atom 4, .list #[3, 11, 12]],
    root := 13 }
#guard symEval _mapLookupDupExpr [] symDefaultFuel defaultIntTy 13
       == SymOutcome.sym (.ctor "Some".toUTF8 #[.const (.int 99)])

private def _mapLookupNoneExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Map".toUTF8, Leaf.name "lookup".toUTF8,
                Leaf.name "map".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[10]), Leaf.intLit false .dec (ByteArray.mk #[5])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 4, .atom 5, .list #[4, 5],
               .atom 3, .list #[7, 6], .atom 6, .list #[3, 8, 9]],
    root := 10 }
#guard symEval _mapLookupNoneExpr [] symDefaultFuel defaultIntTy 10
       == SymOutcome.sym (.ctor "None".toUTF8 #[])

-- SET.INSERT member-op coverage: `((. Set insert) (set 1 3) 2)` → canonical set `[1,2,3]` (sort-inserted).
private def _setInsertExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Set".toUTF8, Leaf.name "insert".toUTF8,
                Leaf.name "set".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[3]), Leaf.intLit false .dec (ByteArray.mk #[2])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .atom 5,
               .list #[4, 5, 6], .atom 6, .list #[3, 7, 8]],
    root := 9 }
#guard symEval _setInsertExpr [] symDefaultFuel defaultIntTy 9
       == SymOutcome.sym (.ctor "set".toUTF8 #[.const (.int 1), .const (.int 2), .const (.int 3)])

-- SET.OF construction coverage (v-cdz-smith #7371/#7412 residual root-cause): `((. Set of) (list 6 0 6))`
-- → canonical set `[0,6]` (sort + dedup). The SOURCE-side set builder whose absence boundaried the whole
-- Set-op differential while the `--target cadenza` roundtrip (a setCtor-headed literal) already folded.
private def _setOfExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Set".toUTF8, Leaf.name "of".toUTF8,
                Leaf.name "list".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[6]),
                Leaf.intLit false .dec (ByteArray.mk #[0])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .atom 5, .atom 4,
               .list #[4, 5, 6, 7], .list #[3, 8]],
    root := 9 }
#guard symEval _setOfExpr [] symDefaultFuel defaultIntTy 9
       == SymOutcome.sym (.ctor "set".toUTF8 #[.const (.int 0), .const (.int 6)])

-- SET.INTERSECTION coverage (v-cdz-smith #7506 closed loop): `((. Set intersection) (set 1 2 3) (set 2 3 4))`
-- → `[2,3]` (elements in both). COMMUTATIVE.
private def _setIntersectionExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Set".toUTF8, Leaf.name "intersection".toUTF8,
                Leaf.name "set".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[2]), Leaf.intLit false .dec (ByteArray.mk #[3]),
                Leaf.intLit false .dec (ByteArray.mk #[4])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .atom 5, .atom 6,
               .list #[4, 5, 6, 7], .atom 3, .atom 5, .atom 6, .atom 7, .list #[9, 10, 11, 12],
               .list #[3, 8, 13]],
    root := 14 }
#guard symEval _setIntersectionExpr [] symDefaultFuel defaultIntTy 14
       == SymOutcome.sym (.ctor "set".toUTF8 #[.const (.int 2), .const (.int 3)])

-- SET.DIFFERENCE coverage: `((. Set difference) (set 1 2 3) (set 2 3 4))` → `[1]` (in `a` not in `b`).
-- NON-COMMUTATIVE — pins the a\b arg ORDER (a swap would give `[4]`, a false verdict).
private def _setDifferenceExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Set".toUTF8, Leaf.name "difference".toUTF8,
                Leaf.name "set".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[2]), Leaf.intLit false .dec (ByteArray.mk #[3]),
                Leaf.intLit false .dec (ByteArray.mk #[4])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .atom 5, .atom 6,
               .list #[4, 5, 6, 7], .atom 3, .atom 5, .atom 6, .atom 7, .list #[9, 10, 11, 12],
               .list #[3, 8, 13]],
    root := 14 }
#guard symEval _setDifferenceExpr [] symDefaultFuel defaultIntTy 14
       == SymOutcome.sym (.ctor "set".toUTF8 #[.const (.int 1)])

-- MAP.REMOVE member-op coverage: `((. Map remove) (map (1 10) (2 20)) 1)` → `[(2,20)]` (key 1 dropped).
private def _mapRemoveExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Map".toUTF8, Leaf.name "remove".toUTF8,
                Leaf.name "map".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[10]), Leaf.intLit false .dec (ByteArray.mk #[2]),
                Leaf.intLit false .dec (ByteArray.mk #[20])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 4, .atom 5, .list #[4, 5],
               .atom 6, .atom 7, .list #[7, 8], .atom 3, .list #[10, 6, 9], .atom 4, .list #[3, 11, 12]],
    root := 13 }
#guard symEval _mapRemoveExpr [] symDefaultFuel defaultIntTy 13
       == SymOutcome.sym (.ctor "map".toUTF8 #[.tuple #[.const (.int 2), .const (.int 20)]])

-- INLINE `(do …)` EXPRESSION coverage: `(do (def x 5) (+ x 1))` → binds x=5, value is the last expr → 6.
private def _inlineDoExpr : Module :=
  { leaves := #[Leaf.name "do".toUTF8, Leaf.name "def".toUTF8, Leaf.name "x".toUTF8,
                Leaf.intLit false .dec (ByteArray.mk #[5]), Leaf.name "+".toUTF8,
                Leaf.intLit false .dec (ByteArray.mk #[1])],
    nodes := #[.atom 1, .atom 2, .atom 3, .list #[0, 1, 2], .atom 4, .atom 2, .atom 5,
               .list #[4, 5, 6], .atom 0, .list #[8, 3, 7]],
    root := 9 }
#guard symEval _inlineDoExpr [] symDefaultFuel defaultIntTy 9
       == SymOutcome.sym (.const (.int 6))

-- INLINE `(do …)` TRAP-ELISION: `(do (/ 1 0) 7)` — the discarded `(/ 1 0)` is UNOBSERVED so its trap is
-- ELIDED (skipped, same env); value is the last expr → 7. Faithful to evalDo's discard-non-def semantics.
private def _inlineDoDiscardExpr : Module :=
  { leaves := #[Leaf.name "do".toUTF8, Leaf.name "/".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[0]), Leaf.intLit false .dec (ByteArray.mk #[7])],
    nodes := #[.atom 1, .atom 2, .atom 3, .list #[0, 1, 2], .atom 4, .atom 0, .list #[5, 3, 4]],
    root := 6 }
#guard symEval _inlineDoDiscardExpr [] symDefaultFuel defaultIntTy 6
       == SymOutcome.sym (.const (.int 7))

-- INLINE `(do …)` LOCAL-FN def + call: `(do (def (double x) (* x 2)) (double 5))` → binds `double` to a
-- localFn, the `(double 5)` call inlines it (x=5, params-only env) → `(* 5 2)` → 10.
private def _doLocalFnExpr : Module :=
  { leaves := #[Leaf.name "do".toUTF8, Leaf.name "def".toUTF8, Leaf.name "double".toUTF8,
                Leaf.name "x".toUTF8, Leaf.name "*".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[2]),
                Leaf.intLit false .dec (ByteArray.mk #[5])],
    nodes := #[.atom 2, .atom 3, .list #[0, 1], .atom 4, .atom 3, .atom 5, .list #[3, 4, 5],
               .atom 1, .list #[7, 2, 6], .atom 2, .atom 6, .list #[9, 10], .atom 0, .list #[12, 8, 11]],
    root := 13 }
#guard symEval _doLocalFnExpr [] symDefaultFuel defaultIntTy 13
       == SymOutcome.sym (.const (.int 10))

-- do LOCAL-FN CAPTURE (inc-2): `(do (def n 10) (def (addN x) (+ x n)) (addN 5))` — `addN` CLOSES over the
-- enclosing binding `n`; the call inlines in `params ++ cap` (x=5, n=10 captured) → `(+ 5 10)` → 15.
private def _doLocalFnCaptureExpr : Module :=
  { leaves := #[Leaf.name "do".toUTF8, Leaf.name "def".toUTF8, Leaf.name "n".toUTF8,
                Leaf.intLit false .dec (ByteArray.mk #[10]), Leaf.name "addN".toUTF8,
                Leaf.name "x".toUTF8, Leaf.name "+".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[5])],
    nodes := #[.atom 1, .atom 2, .atom 3, .list #[0, 1, 2], .atom 4, .atom 5, .list #[4, 5],
               .atom 6, .atom 5, .atom 2, .list #[7, 8, 9], .atom 1, .list #[11, 6, 10],
               .atom 4, .atom 7, .list #[13, 14], .atom 0, .list #[16, 3, 12, 15]],
    root := 17 }
#guard symEval _doLocalFnCaptureExpr [] symDefaultFuel defaultIntTy 17
       == SymOutcome.sym (.const (.int 15))

-- HIGHER-ORDER (v-cdz-smith cluster B): a `(fn …)` LAMBDA passed to a local fn and applied —
-- `(do (def (apply f x) (f x)) (apply (fn (y) (+ y 15)) 9))` → 24. `apply` binds to a localFn; the arg
-- `(fn (y) (+ y 15))` now symEvals to a `.localFn` closure (the new `fn` arm) instead of `cannotProve`;
-- inlining `apply` binds `f`→that closure, `x`→9, then `(f x)` inlines the closure (y→9) → `(+ 9 15)` → 24.
private def _higherOrderLambdaExpr : Module :=
  { leaves := #[Leaf.name "do".toUTF8, Leaf.name "def".toUTF8, Leaf.name "apply".toUTF8,
                Leaf.name "f".toUTF8, Leaf.name "x".toUTF8, Leaf.name "fn".toUTF8, Leaf.name "y".toUTF8,
                Leaf.name "+".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[15]),
                Leaf.intLit false .dec (ByteArray.mk #[9])],
    nodes := #[.atom 2, .atom 3, .atom 4, .list #[0, 1, 2], .atom 3, .atom 4, .list #[4, 5],
               .atom 1, .list #[7, 3, 6], .atom 2, .atom 6, .list #[10], .atom 7, .atom 6, .atom 8,
               .list #[12, 13, 14], .atom 5, .list #[16, 11, 15], .atom 9, .list #[9, 17, 18],
               .atom 0, .list #[20, 8, 19]],
    root := 21 }
#guard symEval _higherOrderLambdaExpr [] symDefaultFuel defaultIntTy 21
       == SymOutcome.sym (.const (.int 24))

-- CURRYING (v-cdz-smith cluster A): a local fn PARTIALLY applied, the closure bound, then completed —
-- `(do (def (pa a b) (- a b)) (let ((f (pa 88))) (f 50)))` → 38. `(pa 88)` supplies 1 of 2 args → a new
-- `.localFn` over the remaining `[b]` with `a`→88 captured; `(f 50)` completes it (full arity → inline)
-- → `(- 88 50)` → 38. Previously the partial `(pa 88)` was `cannotProve "arity mismatch"`.
private def _curryLocalFnExpr : Module :=
  { leaves := #[Leaf.name "do".toUTF8, Leaf.name "def".toUTF8, Leaf.name "pa".toUTF8,
                Leaf.name "a".toUTF8, Leaf.name "b".toUTF8, Leaf.name "-".toUTF8, Leaf.name "let".toUTF8,
                Leaf.name "f".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[88]),
                Leaf.intLit false .dec (ByteArray.mk #[50])],
    nodes := #[.atom 2, .atom 3, .atom 4, .list #[0, 1, 2], .atom 5, .atom 3, .atom 4, .list #[4, 5, 6],
               .atom 1, .list #[8, 3, 7], .atom 7, .atom 2, .atom 8, .list #[11, 12], .list #[10, 13],
               .list #[14], .atom 7, .atom 9, .list #[16, 17], .atom 6, .list #[19, 15, 18],
               .atom 0, .list #[21, 9, 20]],
    root := 22 }
#guard symEval _curryLocalFnExpr [] symDefaultFuel defaultIntTy 22
       == SymOutcome.sym (.const (.int 38))

-- CURRY CHAIN (v-cdz-smith next-tier, NON-NAME head): `(do (def (pa3 a b c) (+ (+ a b) c)) (((pa3 41) 7) 38))`
-- → 86. Each application head is itself an application: `(pa3 41)`→closure[b,c], `((pa3 41) 7)`→closure[c]
-- (non-name head symEval'd), `(((pa3 41) 7) 38)`→full→inline → `(+ (+ 41 7) 38)` → 86. Previously the
-- non-name application head was `cannotProve "non-name head"`.
private def _curryChainExpr : Module :=
  { leaves := #[Leaf.name "do".toUTF8, Leaf.name "def".toUTF8, Leaf.name "pa3".toUTF8,
                Leaf.name "a".toUTF8, Leaf.name "b".toUTF8, Leaf.name "c".toUTF8, Leaf.name "+".toUTF8,
                Leaf.intLit false .dec (ByteArray.mk #[41]), Leaf.intLit false .dec (ByteArray.mk #[7]),
                Leaf.intLit false .dec (ByteArray.mk #[38])],
    nodes := #[.atom 2, .atom 3, .atom 4, .atom 5, .list #[0, 1, 2, 3], .atom 6, .atom 3, .atom 4,
               .list #[5, 6, 7], .atom 6, .atom 5, .list #[9, 8, 10], .atom 1, .list #[12, 4, 11],
               .atom 2, .atom 7, .list #[14, 15], .atom 8, .list #[16, 17], .atom 9, .list #[18, 19],
               .atom 0, .list #[21, 13, 20]],
    root := 22 }
#guard symEval _curryChainExpr [] symDefaultFuel defaultIntTy 22
       == SymOutcome.sym (.const (.int 86))

-- TRY success-unwrap: `(try (Ok 5))` → 5 (the `?` operator on a concrete Ok). leaves 0:try 1:Ok 2:(5).
private def _tryOkExpr : Module :=
  { leaves := #[Leaf.name "try".toUTF8, Leaf.name "Ok".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[5])],
    nodes := #[.atom 1, .atom 2, .list #[0, 1], .atom 0, .list #[3, 2]],
    root := 4 }
#guard symEval _tryOkExpr [] symDefaultFuel defaultIntTy 4
       == SymOutcome.sym (.const (.int 5))

-- TOP-LEVEL VALUE-DEF bare reference: `(do (def k 7) (def (main) (+ k 1)) (export main))` — main's body
-- references the top-level constant `k` bare; symEval resolves it to k's body (7) → `(+ 7 1)` → 8.
private def _topDefRefProg : Module :=
  { leaves := #[Leaf.name "do".toUTF8, Leaf.name "def".toUTF8, Leaf.name "k".toUTF8,
                Leaf.intLit false .dec (ByteArray.mk #[7]), Leaf.name "main".toUTF8,
                Leaf.name "+".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]), Leaf.name "export".toUTF8],
    nodes := #[.atom 1, .atom 2, .atom 3, .list #[0, 1, 2], .atom 4, .list #[4], .atom 5, .atom 2, .atom 6,
               .list #[6, 7, 8], .atom 1, .list #[10, 5, 9], .atom 7, .atom 4, .list #[12, 13],
               .atom 0, .list #[15, 3, 11, 14]],
    root := 16 }
#guard symEvalMain _topDefRefProg == SymOutcome.sym (.const (.int 8))

-- FLOAT-SOUNDNESS regression guards (bit-faithful valEq/canonSet, NOT IEEE SymExpr-beq): NaN DEDUPES
-- (NaN==NaN under the canonical-bits key), so `(set NaN NaN)` canonicalizes to ONE element. If Set/canon
-- ever regressed to the derived IEEE `==` (where NaN≠NaN), this set would keep 2 → guard fires.
private def _setNanDedupExpr : Module :=
  { leaves := #[Leaf.name "set".toUTF8, Leaf.floatNan],
    nodes := #[.atom 0, .atom 1, .atom 1, .list #[0, 1, 2]], root := 3 }
#guard symEval _setNanDedupExpr [] symDefaultFuel defaultIntTy 3
       == SymOutcome.sym (.ctor "set".toUTF8 #[.const .floatNan])

-- and `Set.contains` finds NaN in `(set NaN)` (valEq NaN NaN = true, bit-faithful). IEEE beq would say false.
private def _setContainsNanExpr : Module :=
  { leaves := #[Leaf.name ".".toUTF8, Leaf.name "Set".toUTF8, Leaf.name "contains".toUTF8,
                Leaf.name "set".toUTF8, Leaf.floatNan],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2], .atom 3, .atom 4, .list #[4, 5],
               .atom 4, .list #[3, 6, 7]], root := 8 }
#guard symEval _setContainsNanExpr [] symDefaultFuel defaultIntTy 8
       == SymOutcome.sym (.const (.bool true))

-- NORMALIZE-level soundness regression guards — pin the two CAUGHT false-proven bugs + a collapse invariant:
-- (1) `x - x` must NOT fold to `0` (#6541): if `x` traps, `x - x` traps, so it is not `0`. Stays symbolic.
#guard normalize (.app "-" #[.var 0, .var 0]) == .app "-" #[.var 0, .var 0]
-- (2) `(if c a a)` must NOT collapse to `a` when the CONDITION may trap — dropping a trapping `c` would
-- unsoundly claim `(if <trapping> a a)` (which traps) equals `a`. Here `c = (/ 1 x)` mayTraps → no collapse.
#guard normalize (.ite (.app "/" #[.const (.int 1), .var 0]) (.var 1) (.var 1))
       == .ite (.app "/" #[.const (.int 1), .var 0]) (.var 1) (.var 1)
-- (3) `(if c v v)` with identical FLOAT branches must NOT collapse (#6533): SymExpr's derived `==` is IEEE
-- (`+0.0 == -0.0`, `NaN ≠ NaN`), so collapsing a float branch via `==` is unsound; `symFloatFree` blocks it.
#guard normalize (.ite (.var 0) (.const (.f64 1.5)) (.const (.f64 1.5)))
       == .ite (.var 0) (.const (.f64 1.5)) (.const (.f64 1.5))
-- (4) FP-0 (v-cdz-smith sampled-FP triage): `(+ (if (mod v0 v0) a a) b)` must NOT fold to a constant —
-- `mod v0 v0` may TRAP (v0=0), so the `if` does not collapse (mayTrap guard) and the `+` stays symbolic.
-- This intentional conservatism is WHY the symbolic oracle flags the backend's const-fold of this shape
-- (a fold-through-a-trapping-condition the sampler can miss at v0=0). Pinning it as CORRECT — do not "fix".
#guard normalize (.app "+" #[.ite (.app "%" #[.var 0, .var 0]) (.const (.int (-1000000))) (.const (.int (-1000000))), .const (.int (-1000000))])
       == .app "+" #[.ite (.app "%" #[.var 0, .var 0]) (.const (.int (-1000000))) (.const (.int (-1000000))), .const (.int (-1000000))]
-- FP-1 first-factor: an int op whose operands are `if`s that fold only under normalize now GROUNDS to a
-- literal (symEval normalizes operands before its width-checked int fold). `(let ((v0 5)) (+ (if (> v0 v0)
-- v0 v0) (if (>= 10 v0) 100 v0)))` = (5) + (100) = 105 — was the FP-1 boundary, now proven.
private def _fp1ProbeExpr : Module :=
  { leaves := #[Leaf.name "let".toUTF8, Leaf.name "v0".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[5]),
                Leaf.name "+".toUTF8, Leaf.name ">".toUTF8, Leaf.name ">=".toUTF8,
                Leaf.intLit false .dec (ByteArray.mk #[10]), Leaf.intLit false .dec (ByteArray.mk #[100]),
                Leaf.name "if".toUTF8],
    nodes := #[.atom 1, .atom 2, .list #[0, 1], .list #[2], .atom 4, .atom 1, .atom 1, .list #[4, 5, 6],
               .atom 8, .atom 1, .atom 1, .list #[8, 7, 9, 10], .atom 5, .atom 6, .atom 1, .list #[12, 13, 14],
               .atom 8, .atom 7, .atom 1, .list #[16, 15, 17, 18], .atom 3, .list #[20, 11, 19],
               .atom 0, .list #[22, 3, 21]],
    root := 23 }
#guard (match symEval _fp1ProbeExpr [] symDefaultFuel defaultIntTy 23 with
        | .sym e => normalize e == .const (.int 105) | _ => false)

-- match on a CONCRETE constructor: `(match (Some 5) ((Some x) x) (None 0))` → binds x=5, takes the Some arm → const 5.
-- leaves 0:match 1:Some 2:(5) 3:x 4:None 5:(0). nodes: 2:(Some 5), 5:(Some x) pat, 7:arm1, 10:arm2, 12:(match …).
private def _matchExpr : Module :=
  { leaves := #[Leaf.name "match".toUTF8, Leaf.name "Some".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[5]),
                Leaf.name "x".toUTF8, Leaf.name "None".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[0])],
    nodes := #[.atom 1, .atom 2, .list #[0, 1], .atom 1, .atom 3, .list #[3, 4], .atom 3, .list #[5, 6],
               .atom 4, .atom 5, .list #[8, 9], .atom 0, .list #[11, 2, 7, 10]],
    root := 12 }
#guard symEval _matchExpr [] symDefaultFuel defaultIntTy 12 == SymOutcome.sym (.const (.int 5))

-- user-sum construction. NEWTYPE erases: `(type Cached (Mk Int64))` + `(main)=(Mk 7)` → the payload 7 (no tag).
private def _newtypeProg : Module :=
  { leaves := #[Leaf.name "do".toUTF8, Leaf.name "type".toUTF8, Leaf.name "Cached".toUTF8, Leaf.name "Mk".toUTF8,
                Leaf.name "Int64".toUTF8, Leaf.name "def".toUTF8, Leaf.name "main".toUTF8,
                Leaf.intLit false .dec (ByteArray.mk #[7]), Leaf.name "export".toUTF8],
    nodes := #[.atom 1, .atom 2, .atom 3, .atom 4, .list #[2, 3], .list #[0, 1, 4],
               .atom 5, .atom 6, .list #[7], .atom 3, .atom 7, .list #[9, 10], .list #[6, 8, 11],
               .atom 8, .atom 6, .list #[13, 14], .atom 0, .list #[16, 5, 12, 15]],
    root := 17 }
#guard symEvalMain _newtypeProg == SymOutcome.sym (.const (.int 7))
-- generic TAGGED variant: `(type E (Num Int64)(Wrap Int64))` (multi-variant) + `(main)=(Num 5)` → .ctor "Num" [5].
private def _userSumProg : Module :=
  { leaves := #[Leaf.name "do".toUTF8, Leaf.name "type".toUTF8, Leaf.name "E".toUTF8, Leaf.name "Num".toUTF8,
                Leaf.name "Int64".toUTF8, Leaf.name "Wrap".toUTF8, Leaf.name "def".toUTF8, Leaf.name "main".toUTF8,
                Leaf.intLit false .dec (ByteArray.mk #[5]), Leaf.name "export".toUTF8],
    nodes := #[.atom 1, .atom 2, .atom 3, .atom 4, .list #[2, 3], .atom 5, .atom 4, .list #[5, 6], .list #[0, 1, 4, 7],
               .atom 6, .atom 7, .list #[10], .atom 3, .atom 8, .list #[12, 13], .list #[9, 11, 14],
               .atom 9, .atom 7, .list #[16, 17], .atom 0, .list #[19, 8, 15, 18]],
    root := 20 }
#guard symEvalMain _userSumProg == SymOutcome.sym (.ctor "Num".toUTF8 #[.const (.int 5)])
-- BARE NULLARY-CTOR reference (v-cdz-smith's exact free-name-20 shape): `(type Color (Red)(Green)(Blue))`
-- + `(main)=Blue` → the bare `Blue` resolves to the nullary ctor value `.ctor "Blue" []` (3-variant enum,
-- not sole → tagged). Ctor specs `(Red)`/`(Green)`/`(Blue)` are arity-0. Drains the free-name-20 bucket.
private def _enumRefProg : Module :=
  { leaves := #[Leaf.name "do".toUTF8, Leaf.name "type".toUTF8, Leaf.name "Color".toUTF8,
                Leaf.name "Red".toUTF8, Leaf.name "Green".toUTF8, Leaf.name "Blue".toUTF8,
                Leaf.name "def".toUTF8, Leaf.name "main".toUTF8, Leaf.name "export".toUTF8],
    nodes := #[.atom 1, .atom 2, .atom 3, .list #[2], .atom 4, .list #[4], .atom 5, .list #[6],
               .list #[0, 1, 3, 5, 7], .atom 7, .list #[9], .atom 5, .atom 6, .list #[12, 10, 11],
               .atom 8, .atom 7, .list #[14, 15], .atom 0, .list #[17, 8, 13, 16]],
    root := 18 }
#guard symEvalMain _enumRefProg == SymOutcome.sym (.ctor "Blue".toUTF8 #[])
-- APPLIED nullary ctor `(Blue)` (0-arg APPLICATION, vs the bare atom above) also → `.ctor "Blue" []` via
-- symCtorConstruct (empty arg list → some #[]). Regression + diagnostic guard for the ctor-arg path.
private def _enumApplyProg : Module :=
  { leaves := #[Leaf.name "do".toUTF8, Leaf.name "type".toUTF8, Leaf.name "Color".toUTF8,
                Leaf.name "Red".toUTF8, Leaf.name "Green".toUTF8, Leaf.name "Blue".toUTF8,
                Leaf.name "def".toUTF8, Leaf.name "main".toUTF8, Leaf.name "export".toUTF8],
    nodes := #[.atom 1, .atom 2, .atom 3, .list #[2], .atom 4, .list #[4], .atom 5, .list #[6],
               .list #[0, 1, 3, 5, 7], .atom 7, .list #[9], .atom 5, .list #[11], .atom 6,
               .list #[13, 10, 12], .atom 8, .atom 7, .list #[15, 16], .atom 0, .list #[18, 8, 14, 17]],
    root := 19 }
#guard symEvalMain _enumApplyProg == SymOutcome.sym (.ctor "Blue".toUTF8 #[])
-- ROUND-TRIP form (v-cdz-smith P'): the cadenza backend lowers a nullary ctor to `(: (Ctor unit) T)` — a
-- unit-PLACEHOLDER payload under a type ascription. `(def (main) (: (Blue unit) Color))` → the ascription
-- unwraps, then `(Blue unit)` is the NULLARY construction (unit arg ignored, per evalVariantCtor) → .ctor "Blue" [].
private def _enumRoundtripProg : Module :=
  { leaves := #[Leaf.name "do".toUTF8, Leaf.name "type".toUTF8, Leaf.name "Color".toUTF8,
                Leaf.name "Red".toUTF8, Leaf.name "Green".toUTF8, Leaf.name "Blue".toUTF8,
                Leaf.name "def".toUTF8, Leaf.name "main".toUTF8, Leaf.name "export".toUTF8,
                Leaf.name ":".toUTF8, Leaf.name "unit".toUTF8],
    nodes := #[.atom 1, .atom 2, .atom 3, .list #[2], .atom 4, .list #[4], .atom 5, .list #[6],
               .list #[0, 1, 3, 5, 7], .atom 5, .atom 10, .list #[9, 10], .atom 2, .atom 9,
               .list #[13, 11, 12], .atom 7, .list #[15], .atom 6, .list #[17, 16, 14],
               .atom 8, .atom 7, .list #[19, 20], .atom 0, .list #[22, 8, 18, 21]],
    root := 23 }
#guard symEvalMain _enumRoundtripProg == SymOutcome.sym (.ctor "Blue".toUTF8 #[])
-- construct-then-MATCH a user sum: `(match (Num 5) ((Num x) x))` binds x=5 via the tagged-variant pattern → 5.
private def _matchUserProg : Module :=
  { leaves := #[Leaf.name "do".toUTF8, Leaf.name "type".toUTF8, Leaf.name "E".toUTF8, Leaf.name "Num".toUTF8,
                Leaf.name "Int64".toUTF8, Leaf.name "Wrap".toUTF8, Leaf.name "def".toUTF8, Leaf.name "main".toUTF8,
                Leaf.name "match".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[5]), Leaf.name "x".toUTF8, Leaf.name "export".toUTF8],
    nodes := #[.atom 1, .atom 2, .atom 3, .atom 4, .list #[2, 3], .atom 5, .atom 4, .list #[5, 6], .list #[0, 1, 4, 7],
               .atom 6, .atom 7, .list #[10], .atom 3, .atom 9, .list #[12, 13], .atom 3, .atom 10, .list #[15, 16],
               .atom 10, .list #[17, 18], .atom 8, .list #[20, 14, 19], .list #[9, 11, 21],
               .atom 11, .atom 7, .list #[23, 24], .atom 0, .list #[26, 8, 22, 25]],
    root := 27 }
#guard symEvalMain _matchUserProg == SymOutcome.sym (.const (.int 5))

-- WIDTH-AWARE integer const-folding. `(+ 100 23)` at the default Int64 fits → folds to 123.
private def _addProg : Module :=
  { leaves := #[Leaf.name "+".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[100]), Leaf.intLit false .dec (ByteArray.mk #[23])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2]], root := 3 }
#guard symEval _addProg [] symDefaultFuel defaultIntTy 3 == SymOutcome.sym (.const (.int 123))
-- SOUNDNESS: `(: (+ 200 100) UInt8)` overflows UInt8 (300 ∉ [0,255]) → evalArithOp TRAPS → NOT folded to
-- 300 (folding it would be a false 'proven'); the operation stays symbolic.
private def _ovfProg : Module :=
  { leaves := #[Leaf.name ":".toUTF8, Leaf.name "+".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[200]),
                Leaf.intLit false .dec (ByteArray.mk #[100]), Leaf.name "UInt8".toUTF8],
    nodes := #[.atom 1, .atom 2, .atom 3, .list #[0, 1, 2], .atom 0, .atom 4, .list #[4, 3, 5]], root := 6 }
#guard symEval _ovfProg [] symDefaultFuel defaultIntTy 6 == SymOutcome.sym (.app "+" #[.const (.int 200), .const (.int 100)])

-- SYMBOLIC-scrutinee match: `(def (main x) (match x ((Some a) a) (None n)))` — x is a symbolic param, so the
-- match becomes a symbolic `.case` (Some-arm binds a to the symbolic payload proj; None-arm → n).
private def _symMatchProg (n : UInt8) : Module :=
  { leaves := #[Leaf.name "do".toUTF8, Leaf.name "def".toUTF8, Leaf.name "main".toUTF8, Leaf.name "x".toUTF8,
                Leaf.name "match".toUTF8, Leaf.name "Some".toUTF8, Leaf.name "a".toUTF8, Leaf.name "None".toUTF8,
                Leaf.intLit false .dec (ByteArray.mk #[n]), Leaf.name "export".toUTF8],
    nodes := #[.atom 1, .atom 2, .atom 3, .list #[1, 2], .atom 3, .atom 5, .atom 6, .list #[5, 6], .atom 6,
               .list #[7, 8], .atom 7, .atom 8, .list #[10, 11], .atom 4, .list #[13, 4, 9, 12],
               .list #[0, 3, 14], .atom 9, .atom 2, .list #[16, 17], .atom 0, .list #[19, 15, 18]],
    root := 20 }
-- a symbolic-scrutinee match is a stable symbolic case → a program is PROVEN equivalent to itself for all inputs.
#guard equivMain (_symMatchProg 0) (_symMatchProg 0) == EquivVerdict.proven
-- two matches differing only in the None-arm body are NOT proven (the symbolic cases differ) — no false proven.
#guard equivMain (_symMatchProg 0) (_symMatchProg 1) == EquivVerdict.cannotProve "normalized-but-different"

-- FLOAT32 fold precision (v-cdz-smith fp-0): `(/ (: 10.0 Float32) (: 3.0 Float32))` must fold at f32
-- (the backend rounds per-op to f32), NOT f64. leaves 0:/ 1:: 2:(10.0) 3:Float32 4:(3.0).
private def _f32div : Module :=
  { leaves := #[Leaf.name "/".toUTF8, Leaf.name ":".toUTF8, Leaf.float false 0 (ByteArray.mk #[10]),
                Leaf.name "Float32".toUTF8, Leaf.float false 0 (ByteArray.mk #[3])],
    nodes := #[.atom 1, .atom 2, .atom 3, .list #[0, 1, 2], .atom 1, .atom 4, .atom 3, .list #[4, 5, 6],
               .atom 0, .list #[8, 3, 7]], root := 9 }
#guard symEval _f32div [] symDefaultFuel defaultIntTy 9 == SymOutcome.sym (.const (.f64 (Float.toFloat32 (10.0 / 3.0)).toFloat))
-- contrast: the SAME division without Float32 (bare Float64 literals) folds at f64 (unrounded).
private def _f64div : Module :=
  { leaves := #[Leaf.name "/".toUTF8, Leaf.float false 0 (ByteArray.mk #[10]), Leaf.float false 0 (ByteArray.mk #[3])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2]], root := 3 }
#guard symEval _f64div [] symDefaultFuel defaultIntTy 3 == SymOutcome.sym (.const (.f64 (10.0 / 3.0)))

-- RECORD-pattern match (getting ahead of the fuzzer's record widening, spec 05-compound "record MATCH
-- pattern destructures by field"). Pattern `(record (a x))` at node 4. leaves 0:record 1:a 2:x.
private def _recPat : Module :=
  { leaves := #[Leaf.name "record".toUTF8, Leaf.name "a".toUTF8, Leaf.name "x".toUTF8],
    nodes := #[.atom 1, .atom 2, .list #[0, 1], .atom 0, .list #[3, 2]], root := 4 }
-- against a CONCRETE record `{a=1, b=2}` → binds x to the `a` field (1); partial (ignores b).
#guard symMatchPat _recPat 4 (.record #[("a".toUTF8, .const (.int 1)), ("b".toUTF8, .const (.int 2))])
       == some (some [("x".toUTF8, .const (.int 1))])
-- against a SYMBOLIC scrutinee → binds x to the symbolic field projection `proj scrut a`.
#guard symBindPat _recPat 4 (.var 0) == some [("x".toUTF8, .proj (.var 0) "a".toUTF8)]

-- symExprEqB (reducible structural SymExpr equality; float → false via valEqB): behaves as expected.
#guard symExprEqB (.var 3) (.var 3) == true
#guard symExprEqB (.var 3) (.var 4) == false
#guard symExprEqB (.app "+" #[.var 0, .const (.int 5)]) (.app "+" #[.var 0, .const (.int 5)]) == true
#guard symExprEqB (.app "+" #[.var 0, .const (.int 5)]) (.app "+" #[.var 0, .const (.int 6)]) == false
#guard symExprEqB (.app "+" #[.var 0]) (.app "-" #[.var 0]) == false
#guard symExprEqB (.const (.f64 (0.0 : Float))) (.const (.f64 (0.0 : Float))) == false  -- floats → false (sound)

end Oracle
