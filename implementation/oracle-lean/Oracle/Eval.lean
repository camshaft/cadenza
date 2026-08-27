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

/-- The name a bare-name atom node references, if it is one. -/
def nameOf? (m : Module) (i : Nat) : Option ByteArray :=
  match m.nodes[i]? with
  | some (Node.atom lid) =>
    match m.leaves[lid]? with
    | some (Leaf.name b) => some b
    | _ => none
  | _ => none

/-- The parameter names of a `def` target `(name (: p T)… )` (or `(name p …)`), in order — each param
spec's bound name. -/
def paramSpecNodes (m : Module) (targetId : Nat) : Array Nat :=
  match m.nodes[targetId]? with
  | some (Node.list cs) => cs.extract 1 cs.size
  | _ => #[]

/-- `main`'s parameter-spec node ids + body node, from `(do … (def (main <params…>) BODY) …)`. -/
def mainParamsBody? (m : Module) : Option (Array Nat × Nat) := do
  let root ← m.nodes[m.root]?
  match root with
  | Node.list stmts =>
    stmts.toList.findSome? (fun sid =>
      match asDef? m sid with
      | some dc =>
        match defName? m dc, dc[1]?, dc[dc.size - 1]? with
        | some nm, some targetId, some bodyId =>
          if nm == "main".toUTF8 then some (paramSpecNodes m targetId, bodyId) else none
        | _, _, _ => none
      | none => none)
  | _ => none

/-- The width of a Cadenza integer type — parametric: an UNKNOWN width (an unresolved type variable
`W`, e.g. in generic `(Int W)` code), a KNOWN concrete bit width, or BIG (arbitrary-precision `BigInt`,
never overflows). An `unknown` width makes overflow undecidable, so arithmetic at it is `unsupported`
(a sound coverage-gap) rather than a guess. -/
inductive Width where
  | unknown
  | bits (n : Nat)
  | big
  deriving BEq, Inhabited

/-- The integer type in force for an integer-typed subexpression: signedness + a parametric width
(`Int64` = `(Int 64)`, `UInt8` = `(UInt 8)`, `BigInt` = big, `(Int W)` = unknown). Used ONLY for
overflow-trap decisions — the produced value is width-agnostic (the canonical output form is bare). -/
structure IntTy where
  signed : Bool
  width : Width
  deriving BEq, Inhabited

/-- The model-default integer literal type (unconstrained literal) — `Int64`. -/
def defaultIntTy : IntTy := { signed := true, width := .bits 64 }

/-- A lazily-computed binding outcome. -/
abbrev Thunk := Unit → Outcome

/-- A lexical environment: each name bound LAZILY to a thunk (forced on first use) PLUS its declared
integer type if known (a typed parameter / ascribed binding), innermost first. Laziness is
load-bearing: an UNUSED binding (or one in a short-circuited/dead position) is never forced, so a
binding that would trap does not trap unless its value is actually needed — matching cadenza's
const-fold eliding a dead failing binding. The declared type flows the parameter/binding width into
arithmetic (so a narrow-typed param traps on overflow at its width, not the ambient default). -/
abbrev Env := List (ByteArray × Thunk × Option IntTy)

/-- Look up a name's thunk + declared type (innermost binding wins). -/
def Env.lookup? (env : Env) (name : ByteArray) : Option (Thunk × Option IntTy) :=
  (env.find? (fun e => e.1 == name)).map (fun e => (e.2.1, e.2.2))

/-- Parse an integer type-AST node to an `IntTy`: the aliases `Int8/16/32/64` + `UInt8/16/32/64`, the
parametric `(Int N)` / `(UInt N)` (a NAME width like `(Int W)` → `unknown`), and `BigInt`. A
non-integer type (e.g. `Bool`) → `none`. -/
def parseIntTy? (m : Module) (i : Nat) : Option IntTy :=
  match m.nodes[i]? with
  | some (Node.atom lid) =>
    match m.leaves[lid]? with
    | some (Leaf.name b) =>
      match String.fromUTF8? b with
      | some "BigInt" => some { signed := true, width := .big }
      | some s =>
        if s.startsWith "Int" then (s.drop 3).toNat?.map (fun w => { signed := true, width := .bits w })
        else if s.startsWith "UInt" then (s.drop 4).toNat?.map (fun w => { signed := false, width := .bits w })
        else none
      | none => none
    | _ => none
  | some (Node.list cs) =>
    match m.headName? (Node.list cs) with
    | some h =>
      let signed := h == "Int".toUTF8
      if signed || h == "UInt".toUTF8 then
        match cs[1]? with
        | some wid => match m.nodes[wid]? with
                      | some (Node.atom l) => match m.leaves[l]? with
                        | some (Leaf.intLit false _ mag) => some { signed, width := .bits (Value.beBytesToNat mag) }
                        | some (Leaf.name _) => some { signed, width := .unknown }  -- `(Int W)` width variable
                        | _ => none
                      | _ => none
        | none => none
      else none
    | none => none
  | _ => none

/-- A `def`/`main` parameter spec `(: name T)` (or a bare name) → its bound name + declared integer
type (if `T` is one). The declared type flows the param's width into arithmetic on it. -/
def paramSpec? (m : Module) (specId : Nat) : Option (ByteArray × Option IntTy) :=
  match m.nodes[specId]? with
  | some (Node.list pc) =>  -- `(: name T)`
    match pc[1]? with
    | some nId => (nameOf? m nId).map (fun nm => (nm, (pc[2]?).bind (parseIntTy? m)))
    | none => none
  | some (Node.atom lid) =>  -- a bare-name param
    match m.leaves[lid]? with | some (Leaf.name b) => some (b, none) | _ => none
  | none => none

/-- Evaluate a binary integer operator, trapping on overflow / divide-by-zero per `ty`. Division and
remainder truncate toward zero (matching the checked wasm `i64.div_s`/`rem_s` the compiler emits). An
`unknown` width makes overflow undecidable → `unsupported` (a sound coverage-gap, never a guess);
`big` never overflows. -/
def evalArithOp (op : String) (a b : Int) (ty : IntTy) : Outcome :=
  match ty.width with
  | .unknown => .unsupported "eval: arithmetic at an unresolved (unknown) integer width"
  | .big =>
    if op == "/" || op == "%" then
      if b == 0 then .trap "divide by zero"
      else .value (.int (if op == "/" then Int.tdiv a b else Int.tmod a b))
    else .value (.int (if op == "+" then a + b else if op == "-" then a - b else a * b))
  | .bits w =>
    let lo : Int := if ty.signed then -(2 ^ (w - 1)) else 0
    let hi : Int := if ty.signed then 2 ^ (w - 1) else 2 ^ w  -- exclusive upper bound
    let inB : Int → Bool := fun r => lo ≤ r && r < hi
    if op == "/" || op == "%" then
      if b == 0 then .trap "divide by zero"
      else if op == "/" && ty.signed && a == lo && b == -1 then .trap "overflow"  -- MIN / -1
      else
        let r := if op == "/" then Int.tdiv a b else Int.tmod a b
        if inB r then .value (.int r) else .trap "overflow"
    else
      let r := if op == "+" then a + b else if op == "-" then a - b else a * b
      if inB r then .value (.int r) else .trap "overflow"

/-- The recognized binary arithmetic operator heads. -/
def arithOps : List String := ["+", "-", "*", "/", "%"]

/-- The recognized binary ORDERING operator heads (three-way relational, spec §A Total Order). -/
def cmpOps : List String := ["<", ">", "<=", ">="]

/-- Lexicographic comparison of two byte sequences by UNSIGNED byte value (spec §Ordering: a `Bytes`/
`String` value orders content-lexicographically over its unsigned bytes; a proper prefix compares less).
Total; bounded by the shorter length then a length tie-break. -/
partial def cmpBytes (a b : ByteArray) : Ordering :=
  let rec go (i : Nat) : Ordering :=
    if i < a.size then
      if i < b.size then
        let x := a[i]!; let y := b[i]!
        if x < y then .lt else if x > y then .gt else go (i + 1)
      else .gt            -- b is a proper prefix of a → a is greater
    else
      if i < b.size then .lt else .eq   -- a exhausted: a is a prefix (less) or equal
  go 0

/-- The total-order three-way comparison for the ORDERED value types (spec §Ordering Where Offered Is
Total): integers numerically, `Bool` with false < true, `String`/`Char` content-lexicographically. A
float (no total order — IEEE partial order) or a compound/unmodeled value → `none` (a sound skip; the
oracle never claims an ordering a type does not offer). -/
def compareVals : Value → Value → Option Ordering
  | .int a, .int b => some (compare a b)
  | .bool a, .bool b => some (compare (a == true) (b == true))  -- false < true
  | .str a, .str b => some (cmpBytes a b)
  | .char a, .char b => some (cmpBytes a b)
  | _, _ => none

/-- Whether a relational operator holds given the three-way `Ordering` of its operands. -/
def cmpHolds (op : String) : Ordering → Bool
  | o => match op with
         | "<" => o == .lt
         | ">" => o == .gt
         | "<=" => o != .gt
         | ">=" => o != .lt
         | _ => false

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
        | some (thunk, _) => thunk ()  -- propagates the binding's value / trap / unsupported / diverges
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
        else if (env.lookup? h).isSome then
          -- the head is a BOUND (shadowed) name, not the builtin constructor/operator — this is an
          -- application of that binding, which the pure-core does not model yet → skip.
          .unsupported "eval: head is a bound/shadowed name (application not modeled)"
        else if h == "Some".toUTF8 then evalUnaryCtor m env fuel children Value.some
        else if h == "Ok".toUTF8 then evalUnaryCtor m env fuel children Value.ok
        else if h == "Err".toUTF8 then evalUnaryCtor m env fuel children Value.err
        else if h == "None".toUTF8 then Outcome.value Value.none
        else if h == "tuple".toUTF8 then evalSeqCtor m env fuel children Value.tuple
        else if h == "list".toUTF8 then evalSeqCtor m env fuel children Value.list
        else match String.fromUTF8? h with
             | some hs => if arithOps.contains hs then evalArith m env ty fuel hs children
                          else if hs == "=" then evalEq m env fuel children
                          else if cmpOps.contains hs then evalCmp m env fuel hs children
                          else if hs == "not" then evalNot m env fuel children
                          else if hs == "and" then evalAndOr m env fuel children true
                          else if hs == "or" then evalAndOr m env fuel children false
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
                extend ((nm, (fun _ => evalNode m captured defaultIntTy fuel vId), none) :: env) rest
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
      -- the op's width comes from an operand's ascription OR a bound (param) variable's declared
      -- type, if either is present; else the ambient type
      let operandTyIn := fun (i : Nat) =>
        (operandTy? m i).orElse (fun _ => (nameOf? m i).bind (fun nm => (env.lookup? nm).bind (·.2)))
      let opTy := ((operandTyIn aId).orElse (fun _ => operandTyIn bId)).getD ty
      -- Evaluate BOTH operands and combine by precedence: unsupported > diverges > trap > value. An
      -- UNMODELED operand (unsupported) wins over a sibling's trap — so we skip (never claim a trap we
      -- are unsure of), which is what keeps a `try`/short-circuit case a coverage-gap rather than a
      -- spurious trap.
      let oa := evalNode m env opTy fuel aId
      let ob := evalNode m env opTy fuel bId
      match oa, ob with
      | .unsupported r, _ => .unsupported r
      | _, .unsupported r => .unsupported r
      | .diverges, _ => .diverges
      | _, .diverges => .diverges
      | .trap t, _ => .trap t
      | _, .trap t => .trap t
      | .value (.int a), .value (.int b) => evalArithOp op a b opTy
      | _, _ => .unsupported "eval: non-integer operand to arithmetic"
  | _, _ => .unsupported s!"eval: malformed {op}"

/-- A unary constructor `(Ctor e)` (Some/Ok/Err): evaluate `e`, wrap the value; a trap/diverges/
unsupported inner outcome propagates. -/
partial def evalUnaryCtor (m : Module) (env : Env) (fuel : Nat) (children : Array Nat)
    (wrap : Value → Value) : Outcome :=
  match children[1]? with
  | some eId =>
    match evalNode m env defaultIntTy fuel eId with
    | .value v => .value (wrap v)
    | other => other
  | none => .unsupported "eval: malformed unary constructor"

/-- A sequence constructor `(tuple e…)` / `(list e…)`: evaluate each element left-to-right (a
non-value element propagates), wrap the collected values. -/
partial def evalSeqCtor (m : Module) (env : Env) (fuel : Nat) (children : Array Nat)
    (wrap : Array Value → Value) : Outcome :=
  let rec go (js : List Nat) (acc : Array Value) : Outcome :=
    match js with
    | [] => .value (wrap acc)
    | j :: rest =>
      match evalNode m env defaultIntTy fuel j with
      | .value v => go rest (acc.push v)
      | other => other
  go (children.extract 1 children.size).toList #[]

/-- Evaluate two operands, propagating a non-value outcome by precedence unsupported > diverges > trap
> value (an unmodeled sibling keeps a case a coverage-gap, never a spurious result); on two values,
apply `k`. Shared by `=` and the ordering operators. -/
partial def evalBinValues (m : Module) (env : Env) (fuel : Nat) (aId bId : Nat)
    (k : Value → Value → Outcome) : Outcome :=
  let oa := evalNode m env defaultIntTy fuel aId
  let ob := evalNode m env defaultIntTy fuel bId
  match oa, ob with
  | .unsupported r, _ => .unsupported r
  | _, .unsupported r => .unsupported r
  | .diverges, _ => .diverges
  | _, .diverges => .diverges
  | .trap t, _ => .trap t
  | _, .trap t => .trap t
  | .value va, .value vb => k va vb

/-- `(= a b)` — structural equality (spec §Equality Is Structural: value equality agrees with the
canonical byte form). Modeled by the `Value` domain's structural `BEq`, which is byte-canonical by
construction. A float operand is unmodeled → propagates `unsupported` (sound skip). -/
partial def evalEq (m : Module) (env : Env) (fuel : Nat) (children : Array Nat) : Outcome :=
  match children[1]?, children[2]? with
  | some aId, some bId =>
    if children.size != 3 then .unsupported "eval: = expects 2 operands"
    else evalBinValues m env fuel aId bId (fun va vb => .value (.bool (va == vb)))
  | _, _ => .unsupported "eval: malformed ="

/-- `(< a b)` / `(> …)` / `(<= …)` / `(>= …)` — a relational operator via the type's three-way total
order (`compareVals`). An unordered/unmodeled operand type (float, compound, unit) → `unsupported`. -/
partial def evalCmp (m : Module) (env : Env) (fuel : Nat) (op : String) (children : Array Nat) : Outcome :=
  match children[1]?, children[2]? with
  | some aId, some bId =>
    if children.size != 3 then .unsupported s!"eval: {op} expects 2 operands"
    else evalBinValues m env fuel aId bId (fun va vb =>
      match compareVals va vb with
      | some ord => .value (.bool (cmpHolds op ord))
      | none => .unsupported "eval: ordering on a type that offers no total order (float/compound)")
  | _, _ => .unsupported s!"eval: malformed {op}"

/-- `(not a)` — boolean negation; a non-boolean value is a typecheck error we do not model → skip. -/
partial def evalNot (m : Module) (env : Env) (fuel : Nat) (children : Array Nat) : Outcome :=
  match children[1]? with
  | some aId =>
    if children.size != 2 then .unsupported "eval: not expects 1 operand"
    else match evalNode m env defaultIntTy fuel aId with
         | .value (.bool b) => .value (.bool (!b))
         | .value _ => .unsupported "eval: not of a non-boolean (typecheck not modeled)"
         | other => other
  | none => .unsupported "eval: malformed not"

/-- `(and a b)` / `(or a b)` — the SHORT-CIRCUITING boolean connectives (spec §Boolean Connectives
Short-Circuit): `and` evaluates the right operand only when the left is true, `or` only when the left
is false — so a connective shields a trapping/unmodeled right operand exactly as an unselected `if`
branch does (soundness-load-bearing: a right operand that would trap or is unmodeled is NOT forced when
the left decides the result). `isAnd` selects the connective. -/
partial def evalAndOr (m : Module) (env : Env) (fuel : Nat) (children : Array Nat) (isAnd : Bool) : Outcome :=
  match children[1]?, children[2]? with
  | some aId, some bId =>
    if children.size != 3 then .unsupported "eval: and/or expects 2 operands"
    else match evalNode m env defaultIntTy fuel aId with
         | .value (.bool l) =>
           -- short-circuit: `and` on false → false; `or` on true → true (right operand not evaluated)
           if l == !isAnd then .value (.bool l)
           else match evalNode m env defaultIntTy fuel bId with
                | .value (.bool r) => .value (.bool r)
                | .value _ => .unsupported "eval: and/or right operand is non-boolean"
                | other => other
         | .value _ => .unsupported "eval: and/or left operand is non-boolean"
         | other => other
  | _, _ => .unsupported "eval: malformed and/or"
end

/-- Evaluate the program's `main` body, or `unsupported` if the program shape is not the modeled
`(do (def (main) BODY) (export main))`. -/
def evalMain (m : Module) (fuel : Nat) : Outcome :=
  match mainBody? m with
  | some b => evalNode m [] defaultIntTy fuel b
  | none => .unsupported "eval: program is not a (do (def (main) BODY) (export main)) form"

end Eval

/-- STAGE 2 — run a trial against the program: bind the call arguments to `main`'s parameters (with
each param's declared integer type, so narrow-typed params trap at their width) and evaluate its body.
For the no-argument trial this is a nullary `main` with an empty env. -/
def execute (m : Ast.Module) (args : Array Value) : Outcome :=
  match Eval.mainParamsBody? m with
  | some (specs, bodyId) =>
    if specs.size != args.size then
      .unsupported s!"execute: arity mismatch ({specs.size} params, {args.size} args)"
    else
      -- bind each already-evaluated arg value under its parameter name + declared type
      let bindings := (specs.zip args).filterMap (fun (specId, v) =>
        (Eval.paramSpec? m specId).map (fun (nm, ty) =>
          (nm, (fun _ => Outcome.value v : Eval.Thunk), ty)))
      if bindings.size != specs.size then
        .unsupported "execute: a parameter spec is malformed"
      else
        Eval.evalNode m bindings.toList Eval.defaultIntTy Eval.defaultFuel bodyId
  | none => .unsupported "execute: program is not a (do (def (main …) BODY) (export main)) form"

/-- STAGE 1 — const-evaluate a closed program to its minimal form (grades a bare `(input E)` case).
Equal to `execute` with no arguments (stage parity holds by construction). -/
def reduce (m : Ast.Module) : Outcome := execute m #[]

end Oracle
