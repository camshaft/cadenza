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
  deriving BEq, Inhabited

/-- The symbolic OUTCOME of evaluating a program with symbolic inputs. `cannotProve` records WHY so the
caller can distinguish an incompleteness-boundary gap from a strong (normalized-but-different) lead. -/
inductive SymOutcome where
  | sym (e : SymExpr)
  | cannotProve (reason : String)
  deriving BEq, Inhabited

/-- Constant-fold an operator applied to fully-CONSTANT operands, iff the fold is SOUND independent of
integer width (the symbolic evaluator does not yet track width). So this folds ONLY operators that can
never overflow / trap: COMPARISONS over integer constants (`< > <= >=`, total on `Int`), value EQUALITY
(`=`), and BOOLEAN ops (`and`/`or`/`not`) over boolean constants. Returns `none` (leave symbolic) for a
non-constant operand, a non-integer comparison, or ARITHMETIC (`+ - * / %`) — arithmetic folding needs
width/overflow-trap-aware semantics (T2.0d); folding it here could produce a value where the program
traps = a FALSE "proven", the one outcome worse than `cannotProve`. -/
def foldConst? (op : String) (args : Array SymExpr) : Option Value :=
  let consts := args.map (fun a => match a with | .const v => some v | _ => none)
  if consts.any (·.isNone) then none
  else match op, consts.filterMap id with
    | "=",  #[a, b] => some (.bool (Value.valueEqSpec a b))
    | "<",  #[.int x, .int y] => some (.bool (decide (x < y)))
    | ">",  #[.int x, .int y] => some (.bool (decide (x > y)))
    | "<=", #[.int x, .int y] => some (.bool (decide (x ≤ y)))
    | ">=", #[.int x, .int y] => some (.bool (decide (x ≥ y)))
    | "not", #[.bool b] => some (.bool (!b))
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
partial def mayTrap : SymExpr → Bool
  | .var _ => false
  | .const _ => false
  | .app op args => arithOps.contains op || bitwiseOps.contains op || args.any mayTrap
  | .ite c t e => mayTrap c || mayTrap t || mayTrap e

/-- Canonicalize a symbolic expression by SOUND rewrites only: recurse into subterms; SOUND constant
folding of comparison/boolean ops (`foldConst?`); an `if` on a (now possibly-folded) constant boolean
selects its branch; an `if` whose branches are identical collapses. Deliberately does NOT fold or
reassociate ARITHMETIC (needs width/overflow-trap-aware semantics — T2.0d; an unsound fold = a FALSE
"proven"). Folding a comparison/boolean CONDITION composes with `if`-selection to prove more of an
optimizer's branch-elimination rewrites. -/
partial def normalize : SymExpr → SymExpr
  | .var n => .var n
  | .const v => .const v
  | .app op args =>
    let args' := args.map normalize
    match foldConst? op args' with
    | some v => .const v
    | none => .app op args'
  | .ite c t e =>
    match normalize c with
    | .const (.bool true) => normalize t
    | .const (.bool false) => normalize e
    | c' =>
      let t' := normalize t
      let e' := normalize e
      -- collapse identical branches ONLY when the condition can't trap (dropping a trapping condition
      -- would unsoundly claim `(if <trapping-c> a a)` — which traps — equal to `a`).
      if t' == e' && !mayTrap c' then t' else .ite c' t' e'

/-- A symbolic environment: each program parameter name bound to its symbolic variable. -/
abbrev SymEnv := List (ByteArray × SymExpr)

/-- Symbolically evaluate the node `i` under `senv` (params → symbolic vars). Covers the ANALYZABLE SCALAR
FRAGMENT: a bound parameter → its var; a scalar literal → `const`; `(if c t e)` → `ite`; a `(: e T)`
ascription → its value (type carried structurally — both programs ascribe the same, and Rational grounding
etc. is a future increment); an arithmetic/comparison/boolean operator → `app`. Everything else — `let`,
match/sum, collections, calls, recursion — is the incompleteness boundary → `cannotProve` (honest; degrade
to the sampled differential there). Sound: never invents a value for an unmodeled construct. -/
partial def symEval (m : Module) (senv : SymEnv) (i : Nat) : SymOutcome :=
  match m.nodes[i]? with
  | some (Node.atom lid) =>
    match m.leaves[lid]? with
    | some (Leaf.name b) =>
      match senv.find? (fun p => p.1 == b) with
      | some (_, e) => .sym e
      | none => .cannotProve "symeval: free name (not a bound parameter)"
    | some l =>
      match Value.ofLeaf l with
      | some v => .sym (.const v)
      | none => .cannotProve "symeval: non-scalar leaf"
    | none => .cannotProve "symeval: leaf index out of range"
  | some (Node.list children) =>
    match m.headName? (Node.list children) with
    | some h =>
      if h == "if".toUTF8 then
        match children[1]?, children[2]?, children[3]? with
        | some cId, some tId, some eId =>
          match symEval m senv cId, symEval m senv tId, symEval m senv eId with
          | .sym c, .sym t, .sym e => .sym (.ite c t e)
          | .cannotProve r, _, _ => .cannotProve r
          | _, .cannotProve r, _ => .cannotProve r
          | _, _, .cannotProve r => .cannotProve r
        | _, _, _ => .cannotProve "symeval: malformed if"
      else if h == ":".toUTF8 then
        match children[1]? with
        | some vId => symEval m senv vId
        | none => .cannotProve "symeval: malformed ascription"
      else match String.fromUTF8? h with
        | some hs =>
          if arithOps.contains hs || cmpOps.contains hs || hs == "=" || hs == "and" || hs == "or" || hs == "not" then
            let outs := (children.extract 1 children.size).map (fun c => symEval m senv c)
            match outs.findSome? (fun o => match o with | .cannotProve r => some r | .sym _ => none) with
            | some r => .cannotProve r
            | none => .sym (.app hs (outs.map (fun o => match o with | .sym e => e | .cannotProve _ => .const .unit)))
          else .cannotProve s!"symeval: operator/construct '{hs}' not yet modeled (boundary)"
        | none => .cannotProve "symeval: non-UTF8 head"
    | none => .cannotProve "symeval: non-name head"
  | none => .cannotProve "symeval: node index out of range"

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
    if senv.length == specs.size then symEval m senv bodyId
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
-- SOUNDNESS: ARITHMETIC is NOT folded (needs width/overflow-trap semantics) — left symbolic.
#guard normalize (.app "+" #[.const (.int 2), .const (.int 3)]) == SymExpr.app "+" #[.const (.int 2), .const (.int 3)]
-- two structurally-identical symbolic forms are PROVEN equivalent for all inputs.
#guard symEquiv (.sym (.app "+" #[.var 0, .const (.int 1)])) (.sym (.app "+" #[.var 0, .const (.int 1)])) == EquivVerdict.proven
-- an optimizer that turned `if true then (x+1) else y` into `x+1` is PROVEN equivalent (const-cond select).
#guard symEquiv (.sym (.ite (.const (.bool true)) (.app "+" #[.var 0, .const (.int 1)]) (.var 1)))
                (.sym (.app "+" #[.var 0, .const (.int 1)])) == EquivVerdict.proven
-- genuinely different symbolic forms → cannotProve (never a false "proven").
#guard symEquiv (.sym (.var 0)) (.sym (.var 1)) == EquivVerdict.cannotProve "normalized-but-different"
-- an unmodeled operand poisons the whole side → boundary cannotProve.
#guard symEquiv (.cannotProve "unmodeled") (.sym (.var 0)) == EquivVerdict.cannotProve "boundary: unmodeled"

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

end Oracle
