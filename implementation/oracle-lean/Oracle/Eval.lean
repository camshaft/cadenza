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

/-- A lexical environment: names (as raw bytes) bound to values, innermost first. -/
abbrev Env := List (ByteArray × Value)

/-- Look up a name in the environment (innermost binding wins). -/
def Env.lookup? (env : Env) (name : ByteArray) : Option Value :=
  (env.find? (fun (n, _) => n == name)).map (·.2)

/-- The name a bare-name atom node references, if it is one. -/
def nameOf? (m : Module) (i : Nat) : Option ByteArray :=
  match m.nodes[i]? with
  | some (Node.atom lid) =>
    match m.leaves[lid]? with
    | some (Leaf.name b) => some b
    | _ => none
  | _ => none

mutual
/-- Evaluate a node under `env` to an `Outcome`. Models the pure-core FLOOR: scalar literals,
variable references, `let` (sequential bindings), and `if` (boolean condition). Anything else →
`unsupported`. `fuel` bounds recursion (→ `diverges`). -/
partial def evalNode (m : Module) (env : Env) (fuel : Nat) (i : Nat) : Outcome :=
  match fuel with
  | 0 => .diverges
  | fuel + 1 =>
    match m.nodes[i]? with
    | some (Node.atom lid) =>
      match m.leaves[lid]? with
      | some (Leaf.name b) =>
        -- a bare name: a bound variable, or (unmodeled) a free/prelude name
        match env.lookup? b with
        | some v => .value v
        | none => .unsupported "eval: free name (variable not bound; prelude/global not yet modeled)"
      | some l =>
        match Value.ofLeaf l with
        | some v => .value v
        | none => .unsupported "eval: non-scalar leaf (float/bytes/symbol not yet modeled)"
      | none => .unsupported "eval: atom leaf index out of range"
    | some (Node.list children) =>
      match m.headName? (Node.list children) with
      | some h =>
        if h == "let".toUTF8 then evalLet m env fuel children
        else if h == "if".toUTF8 then evalIf m env fuel children
        else .unsupported "eval: application/operator not yet modeled (L1.1b = let/if/vars/literals)"
      | none => .unsupported "eval: headless list"
    | none => .unsupported "eval: node index out of range"

/-- `(let (bindings) body)`: bind each `(name val)` SEQUENTIALLY (a later binding sees the earlier),
then evaluate `body` in the extended environment. -/
partial def evalLet (m : Module) (env : Env) (fuel : Nat) (children : Array Nat) : Outcome :=
  -- children = [letHead, bindingsListId, bodyId]
  match children[1]?, children[2]? with
  | some bindingsId, some bodyId =>
    match m.nodes[bindingsId]? with
    | some (Node.list pairs) =>
      -- fold the bindings left-to-right, extending env; short-circuit on a non-value binding
      let rec bind (env : Env) (ps : List Nat) : Except Outcome Env := do
        match ps with
        | [] => .ok env
        | pid :: rest =>
          match m.nodes[pid]? with
          | some (Node.list pc) =>
            match pc[0]?, pc[1]? with
            | some nId, some vId =>
              match nameOf? m nId with
              | some nm =>
                match evalNode m env fuel vId with
                | .value v => bind ((nm, v) :: env) rest
                | other => .error other  -- a trap/diverges/unsupported binding value propagates
              | none => .error (.unsupported "eval: let binding target is not a name")
            | _, _ => .error (.unsupported "eval: malformed let binding pair")
          | _ => .error (.unsupported "eval: malformed let binding")
      match bind env pairs.toList with
      | .ok env' => evalNode m env' fuel bodyId
      | .error o => o
    | _ => .unsupported "eval: let bindings are not a list"
  | _, _ => .unsupported "eval: malformed let"

/-- `(if cond then else)`: `cond` must evaluate to a boolean value. -/
partial def evalIf (m : Module) (env : Env) (fuel : Nat) (children : Array Nat) : Outcome :=
  -- children = [ifHead, condId, thenId, elseId]
  match children[1]?, children[2]?, children[3]? with
  | some condId, some thenId, some elseId =>
    match evalNode m env fuel condId with
    | .value (.bool b) => evalNode m env fuel (if b then thenId else elseId)
    | .value _ => .unsupported "eval: if condition is not a boolean (typecheck not modeled)"
    | other => other  -- trap/diverges/unsupported in the condition propagates
  | _, _, _ => .unsupported "eval: malformed if"
end

/-- Evaluate the program's `main` body, or `unsupported` if the program shape is not the modeled
`(do (def (main) BODY) (export main))`. -/
def evalMain (m : Module) (fuel : Nat) : Outcome :=
  match mainBody? m with
  | some b => evalNode m [] fuel b
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
