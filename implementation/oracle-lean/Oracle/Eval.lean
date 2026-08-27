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

/-- A lexical environment: names bound LAZILY to a thunk that computes the binding's outcome when the
variable is first used, innermost first. Laziness is load-bearing: an UNUSED binding (or one in a
short-circuited/dead position) is never forced, so a binding that would trap does not trap unless its
value is actually needed — matching cadenza's const-fold, which elides a dead failing binding. -/
abbrev Env := List (ByteArray × (Unit → Outcome))

/-- Look up a name's thunk (innermost binding wins). -/
def Env.lookup? (env : Env) (name : ByteArray) : Option (Unit → Outcome) :=
  (env.find? (fun (n, _) => n == name)).map (·.2)

/-- The name a bare-name atom node references, if it is one. -/
def nameOf? (m : Module) (i : Nat) : Option ByteArray :=
  match m.nodes[i]? with
  | some (Node.atom lid) =>
    match m.leaves[lid]? with
    | some (Leaf.name b) => some b
    | _ => none
  | _ => none

/-- The integer type in force for an integer-typed subexpression: signedness + a bit width, or
`bits = none` for arbitrary-precision (`BigInt`, never overflows). Cadenza integers are parametric in
width (`(Int width)` / `(UInt width)`; `Int64` = `(Int 64)`). Used ONLY for overflow-trap decisions
during arithmetic — the produced value is width-agnostic (the canonical output form is bare). -/
structure IntTy where
  signed : Bool
  bits : Option Nat
  deriving BEq, Inhabited

/-- The model-default integer literal type (unconstrained literal) — `Int64`. -/
def defaultIntTy : IntTy := { signed := true, bits := some 64 }

/-- Parse an integer type-AST node to an `IntTy`: the aliases `Int8/16/32/64` + `UInt8/16/32/64`, the
parametric `(Int N)` / `(UInt N)`, and `BigInt`. A non-integer type (e.g. `Bool`) → `none`. -/
def parseIntTy? (m : Module) (i : Nat) : Option IntTy :=
  match m.nodes[i]? with
  | some (Node.atom lid) =>
    match m.leaves[lid]? with
    | some (Leaf.name b) =>
      match String.fromUTF8? b with
      | some "BigInt" => some { signed := true, bits := none }
      | some s =>
        if s.startsWith "Int" then (s.drop 3).toNat?.map (fun w => { signed := true, bits := some w })
        else if s.startsWith "UInt" then (s.drop 4).toNat?.map (fun w => { signed := false, bits := some w })
        else none
      | none => none
    | _ => none
  | some (Node.list cs) =>
    -- `(Int N)` / `(UInt N)`: head name + a width int-leaf
    match m.headName? (Node.list cs) with
    | some h =>
      let signed := h == "Int".toUTF8
      let unsigned := h == "UInt".toUTF8
      if signed || unsigned then
        match cs[1]? with
        | some wid => match m.nodes[wid]? with
                      | some (Node.atom l) => match m.leaves[l]? with
                        | some (Leaf.intLit false _ mag) => some { signed, bits := some (Value.beBytesToNat mag) }
                        | _ => none
                      | _ => none
        | none => none
      else none
    | none => none
  | _ => none

/-- Is `n` representable in `ty`? (Arbitrary-precision `BigInt` always is.) -/
def IntTy.inBounds (ty : IntTy) (n : Int) : Bool :=
  match ty.bits with
  | none => true
  | some w =>
    if ty.signed then
      (-(2 ^ (w - 1) : Int)) ≤ n && n < (2 ^ (w - 1) : Int)
    else
      0 ≤ n && n < (2 ^ w : Int)

/-- The most-negative value of a signed width (for the `MIN / -1` overflow trap). -/
def IntTy.minVal (ty : IntTy) : Option Int :=
  match ty.bits with
  | some w => if ty.signed then some (-(2 ^ (w - 1))) else some 0
  | none => none

/-- Evaluate a binary integer operator, trapping on overflow / divide-by-zero per `ty`. Division and
remainder truncate toward zero (matching the checked wasm `i64.div_s`/`rem_s` the compiler emits). -/
def evalArithOp (op : String) (a b : Int) (ty : IntTy) : Outcome :=
  if op == "/" || op == "%" then
    if b == 0 then .trap "divide by zero"
    else if op == "/" && ty.minVal == some a && b == -1 then .trap "overflow"  -- MIN / -1
    else
      let r := if op == "/" then Int.tdiv a b else Int.tmod a b
      if ty.inBounds r then .value (.int r) else .trap "overflow"
  else
    let r := if op == "+" then a + b else if op == "-" then a - b else a * b
    if ty.inBounds r then .value (.int r) else .trap "overflow"

/-- The recognized binary arithmetic operator heads. -/
def arithOps : List String := ["+", "-", "*", "/", "%"]

/-- If an operand node is a `(: e T)` ascription with an integer type `T`, that type — so an operation
takes its width from its operands (e.g. `(+ (: v UInt64) (: 0 UInt64))` is UInt64 arithmetic, not the
ambient default). A minimal bottom-up inference for the scalar core. -/
def operandTy? (m : Module) (i : Nat) : Option IntTy :=
  match m.nodes[i]? with
  | some (Node.list cs) =>
    match m.headName? (Node.list cs) with
    | some h => if h == ":".toUTF8 && cs.size ≥ 3 then parseIntTy? m cs[2]! else none
    | none => none
  | _ => none

mutual
/-- Evaluate a node under `env` at expected integer type `ty` to an `Outcome`. Models the pure-core:
scalar literals, variable references, `let`, `if`, `(: e T)` ascription, and binary integer arithmetic
(`+ - * / %`, trapping on overflow / divide-by-zero per the width). Anything else → `unsupported`.
`fuel` bounds recursion (→ `diverges`). -/
partial def evalNode (m : Module) (env : Env) (ty : IntTy) (fuel : Nat) (i : Nat) : Outcome :=
  match fuel with
  | 0 => .diverges
  | fuel + 1 =>
    match m.nodes[i]? with
    | some (Node.atom lid) =>
      match m.leaves[lid]? with
      | some (Leaf.name b) =>
        -- a bare name: force its (lazy) binding, or (unmodeled) a free/prelude name
        match env.lookup? b with
        | some thunk => thunk ()  -- propagates the binding's value / trap / unsupported / diverges
        | none => .unsupported "eval: free name (variable not bound; prelude/global not yet modeled)"
      | some l =>
        match Value.ofLeaf l with
        | some v => .value v
        | none => .unsupported "eval: non-scalar leaf (float/bytes/symbol not yet modeled)"
      | none => .unsupported "eval: atom leaf index out of range"
    | some (Node.list children) =>
      match m.headName? (Node.list children) with
      | some h =>
        if h == "let".toUTF8 then evalLet m env ty fuel children
        else if h == "if".toUTF8 then evalIf m env ty fuel children
        else if h == ":".toUTF8 then evalAscribe m env ty fuel children
        else match String.fromUTF8? h with
             | some hs => if arithOps.contains hs then evalArith m env ty fuel hs children
                          else .unsupported s!"eval: operator/application {hs} not yet modeled"
             | none => .unsupported "eval: non-UTF8 head"
      | none => .unsupported "eval: headless list"
    | none => .unsupported "eval: node index out of range"

/-- `(let (bindings) body)`: bind each `(name val)` SEQUENTIALLY (a later binding sees the earlier),
then evaluate `body`. Binding values are evaluated at the default integer type (their own annotation,
if any, sets it via `(: … )`); the body inherits the enclosing `ty`. -/
partial def evalLet (m : Module) (env : Env) (ty : IntTy) (fuel : Nat) (children : Array Nat) : Outcome :=
  match children[1]?, children[2]? with
  | some bindingsId, some bodyId =>
    match m.nodes[bindingsId]? with
    | some (Node.list pairs) =>
      -- Extend the env LAZILY: each binding a thunk capturing the env-so-far (sequential — a later
      -- binding sees the earlier). A binding is evaluated only when its variable is forced.
      let rec extend (env : Env) (ps : List Nat) : Except String Env :=
        match ps with
        | [] => .ok env
        | pid :: rest =>
          match m.nodes[pid]? with
          | some (Node.list pc) =>
            match pc[0]?, pc[1]? with
            | some nId, some vId =>
              match nameOf? m nId with
              | some nm =>
                let captured := env
                extend ((nm, fun _ => evalNode m captured defaultIntTy fuel vId) :: env) rest
              | none => .error "eval: let binding target is not a name"
            | _, _ => .error "eval: malformed let binding pair"
          | _ => .error "eval: malformed let binding"
      match extend env pairs.toList with
      | .ok env' => evalNode m env' ty fuel bodyId
      | .error msg => .unsupported msg
    | _ => .unsupported "eval: let bindings are not a list"
  | _, _ => .unsupported "eval: malformed let"

/-- `(if cond then else)`: `cond` must evaluate to a boolean value; the taken branch inherits `ty`. -/
partial def evalIf (m : Module) (env : Env) (ty : IntTy) (fuel : Nat) (children : Array Nat) : Outcome :=
  match children[1]?, children[2]?, children[3]? with
  | some condId, some thenId, some elseId =>
    match evalNode m env defaultIntTy fuel condId with
    | .value (.bool b) => evalNode m env ty fuel (if b then thenId else elseId)
    | .value _ => .unsupported "eval: if condition is not a boolean (typecheck not modeled)"
    | other => other
  | _, _, _ => .unsupported "eval: malformed if"

/-- `(: e T)` — evaluate `e` at the integer type `T` names (if `T` is an integer type; otherwise `e`
keeps the enclosing `ty`). This is where a non-default width enters (e.g. `(: (+ a b) Int8)`). -/
partial def evalAscribe (m : Module) (env : Env) (ty : IntTy) (fuel : Nat) (children : Array Nat) : Outcome :=
  match children[1]?, children[2]? with
  | some valId, some tyId => evalNode m env ((parseIntTy? m tyId).getD ty) fuel valId
  | _, _ => .unsupported "eval: malformed ascription"

/-- `(op a b)` for a binary integer operator — evaluate both operands at `ty` and apply, trapping on
overflow / divide-by-zero per the width. -/
partial def evalArith (m : Module) (env : Env) (ty : IntTy) (fuel : Nat) (op : String) (children : Array Nat) : Outcome :=
  match children[1]?, children[2]? with
  | some aId, some bId =>
    if children.size != 3 then .unsupported s!"eval: {op} expects 2 operands"
    else
      -- the op's width comes from an operand's ascription if present, else the ambient type
      let opTy := ((operandTy? m aId).orElse (fun _ => operandTy? m bId)).getD ty
      match evalNode m env opTy fuel aId with
      | .value (.int a) =>
        match evalNode m env opTy fuel bId with
        | .value (.int b) => evalArithOp op a b opTy
        | .value _ => .unsupported "eval: non-integer operand to arithmetic"
        | other => other
      | .value _ => .unsupported "eval: non-integer operand to arithmetic"
      | other => other
  | _, _ => .unsupported s!"eval: malformed {op}"
end

/-- Evaluate the program's `main` body, or `unsupported` if the program shape is not the modeled
`(do (def (main) BODY) (export main))`. -/
def evalMain (m : Module) (fuel : Nat) : Outcome :=
  match mainBody? m with
  | some b => evalNode m [] defaultIntTy fuel b
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
