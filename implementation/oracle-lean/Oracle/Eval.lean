/-
The pure-total-core evaluator — the two stages of design §1.1, built CLEAN-ROOM from `spec/` + the
corpus (never from rcdzc):

  reduce  : Module → Outcome            -- compile-time const-evaluation to minimal form
  execute : Module × args → Outcome     -- runtime execution of an input against the reduced program

Both are pure, total (`Except`-free — every path yields an `Outcome`), deterministic, and fuel-bounded
(`diverges` on exhaustion). A construct outside the modeled subset yields `unsupported` — a sound
coverage-gap the harness skips, never a differential mismatch — so coverage grows monotonically.

L1.1a (this slice) models the pure core's FLOOR: resolve a program `(do (def (main) BODY) (export
main))`, and evaluate a scalar-LITERAL body to its `Value`. Everything else is `unsupported`.
Arithmetic (trapping Int64 add/sub/mul, div-by-zero), `let`, `if`, functions, and match arrive in following
slices. The two stages are separate entry points from the start (§6): `reduce` const-evaluates with no
args; `execute` runs a trial, and for the no-argument trial equals `reduce` (stage parity).
-/
import Oracle.Ast
import Oracle.Value

namespace Oracle

open Oracle.Ast

/-- The evaluator's result (distinct from the wire `Frame.Outcome`): a computed value, a trap, fuel
exhaustion, or an un-modeled construct. -/
inductive Outcome where
  | value (v : Value)
  | trap (kind : String)
  | diverges
  | unsupported (reason : String)
  deriving Inhabited, BEq

namespace Eval

/-- A generous fixed step budget; `reduce`/`execute` bound their work by it and yield `diverges` on
exhaustion (random/looping programs will need this once recursion is modeled). -/
def defaultFuel : Nat := 1000000

/-- The head `Name` bytes of a `List` node, if its first child is an atom → `Name` leaf. -/
def headName? (m : Module) (node : Node) : Option ByteArray := m.headName? node

/-- Is this node the `(def <target> <body>)` form? Returns its child node ids if so. -/
def asDef? (m : Module) (i : Nat) : Option (Array Nat) :=
  match m.nodes[i]? with
  | some (Node.list children) =>
    match headName? m (Node.list children) with
    | some h => if h == "def".toUTF8 then some children else none
    | none => none
  | _ => none

/-- The head name of a `(def <target> …)`'s target, i.e. the defined name (`main`). The target is the
first non-head child, itself a `(name …)` list or a bare name atom. -/
def defName? (m : Module) (defChildren : Array Nat) : Option ByteArray :=
  match defChildren[1]? with
  | some tid =>
    match m.nodes[tid]? with
    | some (Node.atom lid) =>
      match m.leaves[lid]? with
      | some (Leaf.name b) => some b
      | _ => none
    | some (Node.list _) => m.headName? (m.nodes[tid]!)  -- `(main)` or `(main params…)`
    | none => none
  | none => none

/-- Find the body node of `(def (main) BODY …)` in a `(do …)` program: the LAST child of the def whose
target names `main`. (A `def`'s trailing child is its body expression.) -/
def mainBody? (m : Module) : Option Nat := do
  let root ← m.nodes[m.root]?
  match root with
  | Node.list stmts =>
    -- scan statements for a `(def (main) … BODY)`
    let find := stmts.toList.findSome? (fun sid =>
      match asDef? m sid with
      | some dc =>
        match defName? m dc with
        | some nm => if nm == "main".toUTF8 then dc[dc.size - 1]? else none
        | none => none
      | none => none)
    find
  | _ => none

/-- Evaluate a node to an `Outcome`. L1.1a: a scalar-literal atom → its `Value`; anything else →
`unsupported`. `fuel` bounds the (future) recursion. -/
def evalNode (m : Module) (fuel : Nat) (i : Nat) : Outcome :=
  match fuel with
  | 0 => .diverges
  | _ + 1 =>
    match m.nodes[i]? with
    | some (Node.atom lid) =>
      match m.leaves[lid]? with
      | some l =>
        match Value.ofLeaf l with
        | some v => .value v
        | none => .unsupported "eval: non-scalar atom (name/symbol/float/bytes not yet modeled)"
      | none => .unsupported "eval: atom leaf index out of range"
    | some (Node.list _) =>
      .unsupported "eval: compound expression not yet modeled (L1.1a = literals only)"
    | none => .unsupported "eval: node index out of range"

/-- Evaluate the program's `main` body, or `unsupported` if the program shape is not the modeled
`(do (def (main) BODY) (export main))`. -/
def evalMain (m : Module) (fuel : Nat) : Outcome :=
  match mainBody? m with
  | some b => evalNode m fuel b
  | none => .unsupported "eval: program is not a (do (def (main) BODY) (export main)) form"

end Eval

/-- STAGE 1 — const-evaluate a closed program to its minimal form (grades a bare `(input E)` case). -/
def reduce (m : Ast.Module) : Outcome := Eval.evalMain m Eval.defaultFuel

/-- STAGE 2 — run a trial against the program. L1.1a models only the no-argument trial (which equals
`reduce`); a trial WITH arguments needs function application (a later slice) → `unsupported`. -/
def execute (m : Ast.Module) (args : Array Value) : Outcome :=
  if args.isEmpty then Eval.evalMain m Eval.defaultFuel
  else .unsupported "execute: argument application not yet modeled (L1.1a)"

end Oracle
