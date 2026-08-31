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
  else match op, consts.filterMap id with
    | "=",  #[a, b] => some (.bool (Value.valueEqSpec a b))
    | "<",  #[.int x, .int y] => some (.bool (decide (x < y)))
    | ">",  #[.int x, .int y] => some (.bool (decide (x > y)))
    | "<=", #[.int x, .int y] => some (.bool (decide (x ≤ y)))
    | ">=", #[.int x, .int y] => some (.bool (decide (x ≥ y)))
    -- FLOAT arithmetic + comparison: IEEE is TOTAL (overflow → inf/nan, never a trap), so folding float
    -- constants is SOUND with no width/trap tracking (unlike integer arith, which is left deferred). Both
    -- operands must be floats — `asF64?` is `none` for an int, so int arith/comparison is NOT folded here
    -- (int `<` etc. are the `.int`-pattern arms above; int `+ - * /` stay symbolic pending trap-conditions).
    -- REUSE the concrete evaluator's float op (evalFloatOp) — do NOT re-implement float arithmetic, so the
    -- symbolic fold uses byte-identical IEEE semantics to `evalNode`. Fold only when it yields a value.
    | "+",  #[a, b] => (match Value.asF64? a, Value.asF64? b with | some x, some y => (match evalFloatOp op x y with | .value v => some v | _ => none) | _, _ => none)
    | "-",  #[a, b] => (match Value.asF64? a, Value.asF64? b with | some x, some y => (match evalFloatOp op x y with | .value v => some v | _ => none) | _, _ => none)
    | "*",  #[a, b] => (match Value.asF64? a, Value.asF64? b with | some x, some y => (match evalFloatOp op x y with | .value v => some v | _ => none) | _, _ => none)
    | "/",  #[a, b] => (match Value.asF64? a, Value.asF64? b with | some x, some y => (match evalFloatOp op x y with | .value v => some v | _ => none) | _, _ => none)
    | "<",  #[a, b] => (match Value.asF64? a, Value.asF64? b with | some x, some y => some (.bool (x < y)) | _, _ => none)
    | ">",  #[a, b] => (match Value.asF64? a, Value.asF64? b with | some x, some y => some (.bool (x > y)) | _, _ => none)
    | "<=", #[a, b] => (match Value.asF64? a, Value.asF64? b with | some x, some y => some (.bool (x ≤ y)) | _, _ => none)
    | ">=", #[a, b] => (match Value.asF64? a, Value.asF64? b with | some x, some y => some (.bool (x ≥ y)) | _, _ => none)
    -- (`not` is handled by the leading size-dispatch above.)
    | "and", vs => if vs.all (· == .bool true) then some (.bool true)
                   else if vs.any (· == .bool false) then some (.bool false) else none
    | "or",  vs => if vs.any (· == .bool true) then some (.bool true)
                   else if vs.all (· == .bool false) then some (.bool false) else none
    | _, _ => none

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
termination_by e => sizeOf e
decreasing_by
  all_goals simp_wf
  all_goals
    first
      | omega
      | (have h := Array.sizeOf_lt_of_mem x.property; omega)
      | (rcases x with ⟨⟨k, e⟩, hmem⟩; have h := Array.sizeOf_lt_of_mem hmem; simp_all; omega)

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
def normalizeAppIdentities (op : String) (args' : Array SymExpr) : SymExpr :=
  let isI := fun (e : SymExpr) (n : Int) => e == SymExpr.const (Value.int n)
  let isB := fun (e : SymExpr) (b : Bool) => e == SymExpr.const (Value.bool b)
  match op, args' with
  | "+", #[a, b] => if isI b 0 then a else if isI a 0 then b else .app op args'
  -- `x-0→x` PRESERVES the operand (the `int 0` literal forces an int context; `- float int` is ill-typed).
  -- SOUNDNESS: NO `x-x→0` here. `-` is valid on FLOATS too, and `normalize` is type-erased (a `var` may be
  -- float), so `(- x x)` on a float `x` would wrongly fold to `.int 0` — but `x - x` is `.f64 (x-x)`
  -- (NaN for x=NaN/inf; `.f64 0.0 ≠ .int 0` even when finite). It is NOT meaning-preserving, so it is
  -- removed (was an unsound completeness fold). (`x^x→0` is safe: `^` is integer-only, so `^` on floats is
  -- ill-typed and never compiles.)
  | "-", #[a, b] => if isI b 0 then a else .app op args'
  | "*", #[a, b] => if isI b 1 then a else if isI a 1 then b
                    else if isI b 0 && !mayTrap a then SymExpr.const (Value.int 0)
                    else if isI a 0 && !mayTrap b then SymExpr.const (Value.int 0)
                    else .app op args'
  -- DIVISION/MODULO by the literal 1 — divisor 1 is never 0 and never the INT_MIN/-1 overflow case, so
  -- these never trap. `x/1=x` PRESERVES the operand (no guard); `x%1=0` for all x DROPS the dividend →
  -- `!mayTrap` guard. Only the literal-1 divisor is folded (general `/`,`%` stay deferred: trap-conditional).
  | "/", #[a, b] => if isI b 1 then a else .app op args'
  | "%", #[a, b] => if isI b 1 && !mayTrap a then SymExpr.const (Value.int 0) else .app op args'
  | "or", #[a, b] =>
    if isB a true then SymExpr.const (Value.bool true)
    else if isB a false then b
    else if isB b false then a
    else if isB b true && !mayTrap a then SymExpr.const (Value.bool true)
    -- IDEMPOTENCE `x or x → x` (bool companion of `x|x→x`): PRESERVES the operand (both sides evaluate x,
    -- with the same trap), so no `!mayTrap` guard — sound like `x&x→x`/`x|x→x`.
    else if a == b then a
    else .app op args'
  | "and", #[a, b] =>
    if isB a false then SymExpr.const (Value.bool false)
    else if isB a true then b
    else if isB b true then a
    else if isB b false && !mayTrap a then SymExpr.const (Value.bool false)
    else if a == b then a  -- `x and x → x` idempotence (operand-preserving, like `x or x → x`)
    else .app op args'
  -- SOUND BITWISE identities — WIDTH-INDEPENDENT (0 is all-zero bits, `<<`/`>>` by 0 is identity at any
  -- width), the bit-op companions of the integer ones above. `x&0`/`0&x`→0 DROPS the operand → `!mayTrap`
  -- guard; `x|0`/`x^0`/`x<<0`/`x>>0`→x and `x&x`/`x|x`→x PRESERVE the operand (incl. its traps).
  | "&", #[a, b] => if isI b 0 && !mayTrap a then SymExpr.const (Value.int 0)
                    else if isI a 0 && !mayTrap b then SymExpr.const (Value.int 0)
                    else if a == b then a
                    else .app op args'
  | "|", #[a, b] => if isI b 0 then a else if isI a 0 then b else if a == b then a else .app op args'
  -- `x^0`/`0^x`→x PRESERVE the operand; `x^x`→0 (XOR of equal operands is all-zero at ANY width, the
  -- common zeroing idiom; the XOR companion of `x-x→0`/`x&0→0`) DROPS the operand → `!mayTrap` guard.
  | "^", #[a, b] => if isI b 0 then a else if isI a 0 then b
                    else if a == b && !mayTrap a then SymExpr.const (Value.int 0)
                    else .app op args'
  | "<<", #[a, b] => if isI b 0 then a else .app op args'
  | ">>", #[a, b] => if isI b 0 then a else .app op args'
  -- DOUBLE-NEGATION `not (not x) → x`: the two `not`s evaluate `x` once and cancel (bool involution),
  -- with the same trap behavior as `x` — operand-preserving, no guard (matches the identity discipline
  -- above: sound for well-typed bool `x`; an ill-typed `x` never compiles into the differential).
  | "not", #[a] => match a with
                   | .app "not" #[inner] => inner
                   | _ => .app op args'
  | _, _ => .app op args'

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
      | _, _ => if t' == e' && !mayTrap c' && symFloatFree t' then t' else .ite c' t' e'
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
      if h == "Some".toUTF8 || h == "Ok".toUTF8 || h == "Err".toUTF8 then
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
      | none => if b == "None".toUTF8 then .sym (.ctor "None".toUTF8 #[])
                else .cannotProve "symeval: free name (not a bound parameter)"
    | some l =>
      match Value.ofLeaf l with
      | some v => .sym (.const v)
      | none => .cannotProve "symeval: non-scalar leaf"
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
      else if h == "set".toUTF8 then
        -- a SET literal → `.ctor "set"` of the element SymExprs, in SOURCE ORDER. SOUND: two set literals
        -- with the same elements in the same order prove equal; DIFFERING order → normalized-but-different
        -- (never a false `proven`). INCOMPLETE for set REORDERING equality (`(set a b)` vs `(set b a)`) —
        -- canonicalization (sort+dedup by a SymExpr order) is a later increment; but this already lifts a
        -- set-literal program from a `cannotProve` BLIND SPOT to a checked verdict (proven when order matches
        -- the round-trip, as with list). Distinct head from `list`/`tuple`.
        let outs := (children.extract 1 children.size).map (fun c => symEval m senv fuel ty c)
        match outs.findSome? (fun o => match o with | .cannotProve r => some r | .sym _ => none) with
        | some r => .cannotProve r
        | none => .sym (.ctor "set".toUTF8 (outs.map (fun o => match o with | .sym e => e | .cannotProve _ => .const .unit)))
      else if h == "map".toUTF8 then
        -- a MAP literal: each entry is `(k v)` or `(= k v)` (mirrors `evalMapLiteral`'s key/value parse).
        -- Model as `.ctor "map"` of one `.tuple #[key, value]` per entry, in SOURCE ORDER. SOUND: same-order
        -- maps prove equal; differing order → normalized-but-different (never false `proven`). INCOMPLETE for
        -- map key-REORDERING equality (`evalNode` canonicalizes maps sorted-by-key; canonicalization here
        -- needs a SymExpr order — later increment). Still lifts map-literal programs out of the cannotProve
        -- blind spot. A malformed / unmodelable entry sinks the whole map.
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
        | none => .sym (.ctor "map".toUTF8 (entryOuts.map (fun o => match o with | .sym e => e | .cannotProve _ => .const .unit)))
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
              let args := outs.map (fun o => match o with | .sym e => e | .cannotProve _ => .const .unit)
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
              | _, _ => .sym (SymExpr.app hs args)
          else
            -- a call `(f arg…)` to a top-level def `f` (not shadowed by a local): INLINE it — bind each
            -- param to its arg's SymExpr (evaluated in the CALLER env), then symEval the callee body in a
            -- FRESH env of just those params (a top-level def sees only its params + globals), fuel-1. Fuel
            -- bounds recursion: a recursive `f` exhausts it → cannotProve (proving a recursive function's
            -- equivalence needs induction, not modeled). A partial application (arity mismatch) → boundary.
            if (senv.find? (fun p => p.1 == h)).isSome then
              .cannotProve "symeval: head is a local binding (not a top-level call)"
            else match namedParamsBody? m h with
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
              | .sym (.ctor t elems), .sym (.const xv) =>
                if t == "set".toUTF8 then
                  (if elems.all (fun e => match e with | .const _ => true | _ => false) then
                     .sym (.const (.bool (elems.any (fun e => match e with | .const ev => valEq ev xv | _ => false))))
                   else .cannotProve "symeval: Set.contains needs all-concrete elements")
                else .cannotProve "symeval: Set.contains on a non-set value"
              | .cannotProve r, _ => .cannotProve r
              | _, .cannotProve r => .cannotProve r
              | _, _ => .cannotProve "symeval: Set.contains on non-set / non-const query")
           | _, _ => .cannotProve "symeval: malformed Set.contains")
        else .cannotProve "symeval: member-op head not modeled (boundary)"
      | none => .cannotProve "symeval: non-name head"
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
          else if soleNullaryCtor? m cname then .sym (.const .unit)
          else match variantCtorArity? m cname with
            | some ar =>
              if args.size != ar then .cannotProve "symeval: constructor arity mismatch (partial application?)"
              else if ar == 0 then .sym (.ctor cname #[])
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

-- SET literal coverage: `(set 1 2)` → `.ctor "set" [const 1, const 2]` (source order; was cannotProve).
private def _setExpr : Module :=
  { leaves := #[Leaf.name "set".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[2])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2]], root := 3 }
#guard symEval _setExpr [] symDefaultFuel defaultIntTy 3
       == SymOutcome.sym (.ctor "set".toUTF8 #[.const (.int 1), .const (.int 2)])

-- MAP literal coverage: `(map (1 10) (2 20))` → `.ctor "map" [tuple[1,10], tuple[2,20]]` (source order).
private def _mapExpr : Module :=
  { leaves := #[Leaf.name "map".toUTF8, Leaf.intLit false .dec (ByteArray.mk #[1]),
                Leaf.intLit false .dec (ByteArray.mk #[10]), Leaf.intLit false .dec (ByteArray.mk #[2]),
                Leaf.intLit false .dec (ByteArray.mk #[20])],
    nodes := #[.atom 0, .atom 1, .atom 2, .list #[1, 2], .atom 3, .atom 4, .list #[4, 5], .list #[0, 3, 6]],
    root := 7 }
#guard symEval _mapExpr [] symDefaultFuel defaultIntTy 7
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

end Oracle
