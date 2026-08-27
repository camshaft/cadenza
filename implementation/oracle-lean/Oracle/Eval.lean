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

/-- The recognized binary BITWISE / SHIFT operator heads (symbolic + named). -/
def bitwiseOps : List String := ["&", "|", "^", "<<", ">>", "band", "bor", "bxor", "shl", "shr"]

/-- Evaluate a binary bitwise / shift operator on integers, per the width `ty` (derived from the corpus:
`&`/`|`/`^` operate on the two's-complement width-bit pattern; `>>` is ARITHMETIC for a signed type
(floor-division toward −∞: `-256 >> 7 = -2`, `-1 >> 1 = -1`) and logical for unsigned; `<<` is `x·2ⁿ`
range-checked per the width (a runtime out-of-range shift → the `overflow` trap). A shift count `< 0` or
`≥ width` traps `shift count out of range` (→ the `unreachable` kind); the CONST such cases fail-loud at
compile time (CDZ0304) and the checker skips them. `unknown` width → unsupported; on `BigInt`, shifts are
exact and unbounded but bitwise and/or/xor is not modeled (unbounded two's complement). -/
def evalBitOp (op : String) (a b : Int) (ty : IntTy) : Outcome :=
  match ty.width with
  | .unknown => .unsupported "eval: bitwise/shift at an unresolved (unknown) integer width"
  | .big =>
    if op == "<<" || op == "shl" then
      if b < 0 then .unsupported "eval: negative shift count on BigInt" else .value (.int (a * (2 : Int) ^ b.toNat))
    else if op == ">>" || op == "shr" then
      if b < 0 then .unsupported "eval: negative shift count on BigInt" else .value (.int (Int.fdiv a ((2 : Int) ^ b.toNat)))
    else .unsupported "eval: bitwise and/or/xor on BigInt not modeled (unbounded two's complement)"
  | .bits w =>
    let modw : Int := (2 : Int) ^ w
    let pat : Int → Nat := fun x => (((x % modw) + modw) % modw).toNat        -- two's-complement w-bit pattern
    let ofPat : Nat → Int := fun p =>
      let pi : Int := Int.ofNat p
      if ty.signed && pi ≥ (2 : Int) ^ (w - 1) then pi - modw else pi          -- reinterpret the pattern (signed)
    if op == "&" || op == "band" then .value (.int (ofPat (Nat.land (pat a) (pat b))))
    else if op == "|" || op == "bor" then .value (.int (ofPat (Nat.lor (pat a) (pat b))))
    else if op == "^" || op == "bxor" then .value (.int (ofPat (Nat.xor (pat a) (pat b))))
    else if op == "<<" || op == "shl" then
      if b < 0 || b ≥ Int.ofNat w then .trap "shift count out of range"
      else
        let r := a * (2 : Int) ^ b.toNat
        let lo : Int := if ty.signed then -((2 : Int) ^ (w - 1)) else 0
        let hi : Int := if ty.signed then (2 : Int) ^ (w - 1) else (2 : Int) ^ w
        if lo ≤ r && r < hi then .value (.int r) else .trap "overflow"
    else if op == ">>" || op == "shr" then
      if b < 0 || b ≥ Int.ofNat w then .trap "shift count out of range"
      else if ty.signed then .value (.int (Int.fdiv a ((2 : Int) ^ b.toNat)))   -- arithmetic shift
      else .value (.int (Int.ofNat (Nat.shiftRight (pat a) b.toNat)))           -- logical shift
    else .unsupported s!"eval: unknown bitwise op {op}"

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
  | .bytes a, .bytes b => some (cmpBytes a b)   -- Bytes: content-lexicographic over unsigned bytes (spec §329)
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

/-- Surface a deferred element outcome (poison) as an evaluator `Outcome`. -/
def deferredToOutcome : Deferred → Outcome
  | .trap k => .trap k
  | .diverges => .diverges
  | .unsupported r => .unsupported r

/-- Store a non-value element outcome as a `poison` (deferred); a value passes through. Used at compound
CONSTRUCTION so an UNOBSERVED element (never projected, never flowed to the result) never surfaces its
trap/divergence/unmodeled outcome — matching cadenza (spec core-semantics.md #A Trap Occurs When
Observed). -/
def outcomeToValue : Outcome → Value
  | .value v => v
  | .trap k => .poison (.trap k)
  | .diverges => .poison .diverges
  | .unsupported r => .poison (.unsupported r)

/-- SHALLOW observation (a projection reads one field/element): a TOP-LEVEL poison surfaces its
outcome; any other value is returned as-is — a nested compound's inner poisons stay deferred until they
are themselves observed. -/
def observeShallow : Value → Outcome
  | .poison d => deferredToOutcome d
  | v => .value v

/-- DEEP observation (a value flows to the program result / a host call / an equality-or-ordering
comparison that inspects it fully): the FIRST poison anywhere in the value surfaces its outcome; else
the value unchanged. -/
partial def observeDeep (v : Value) : Outcome :=
  match v with
  | .poison d => deferredToOutcome d
  | .some x | .ok x | .err x | .variant _ x =>
    match observeDeep x with | .value _ => .value v | other => other
  | .tuple es | .list es | .set es =>
    match es.findSome? (fun e => match observeDeep e with | .value _ => Option.none | o => Option.some o) with
    | Option.some o => o | Option.none => .value v
  | .record fs =>
    match fs.findSome? (fun kv => match observeDeep kv.2 with | .value _ => Option.none | o => Option.some o) with
    | Option.some o => o | Option.none => .value v
  | .map es =>
    match es.findSome? (fun kv =>
      match observeDeep kv.1 with
      | .value _ => (match observeDeep kv.2 with | .value _ => Option.none | o => Option.some o)
      | o => Option.some o) with
    | Option.some o => o | Option.none => .value v
  | _ => .value v

/-- Prelude SUM constructors modeled as generic `variant` values (name, arity). Sign + Ordering are the
nullary monomorphic prelude sums (rcdzc sums.rs). Option/Result use the dedicated built-in Some/Ok/Err/
None path; Bool renders as scalar true/false; Ast is qualified-only and deferred. -/
def preludeSumCtors : List (String × Nat) :=
  [("Neg", 0), ("Zero", 0), ("Pos", 0), ("Less", 0), ("Equal", 0), ("Greater", 0)]

/-- Per top-level `(type T v1 v2 …)` statement: (typeName, [(ctorName, arity)]). Each variant `vi` is a
list `(Ci τ1…τk)` (arity k; nullary `(Ci)` = 0) or a bare name atom (arity 0); a `(doc …)` is skipped. -/
def variantSpecs (m : Module) (specs : Array Nat) : List (ByteArray × Nat) :=
  specs.toList.filterMap (fun sid =>
    match m.nodes[sid]? with
    | some (Node.list vc) =>
      match m.headName? (Node.list vc) with
      | some h => if h == "doc".toUTF8 then none else some (h, vc.size - 1)
      | none => none
    | some (Node.atom lid) => match m.leaves[lid]? with | some (Leaf.name b) => some (b, 0) | _ => none
    | none => none)

/-- Scan the program's top-level `(type T …)` statements → per-type (typeName, [(ctor, arity)]). -/
def userSumTypes (m : Module) : List (ByteArray × List (ByteArray × Nat)) :=
  match m.nodes[m.root]? with
  | some (Node.list stmts) =>
    stmts.toList.filterMap (fun sid =>
      match m.nodes[sid]? with
      | some (Node.list tc) =>
        match m.headName? (Node.list tc) with
        | some h => if h == "type".toUTF8 then (nameOf? m (tc[1]!)).map (fun tn => (tn, variantSpecs m (tc.extract 2 tc.size))) else none
        | none => none
      | _ => none)
  | _ => []

/-- Top-level `(def (name …) …)` names — a bare ctor name shadowed by such a def is NOT a constructor
(scope-first resolution: def/let/param bind before a bare ctor name; spec-confirmed via corpus 0683). -/
def defNames (m : Module) : List ByteArray :=
  match m.nodes[m.root]? with
  | some (Node.list stmts) =>
    stmts.toList.filterMap (fun sid => (asDef? m sid).bind (fun dc => defName? m dc))
  | _ => []

/-- The arity of a name if it is a modeled generic-`variant` constructor: a prelude Sign/Ordering ctor,
or a scanned USER sum ctor — EXCLUDING (→ `none`, a sound skip): a def-shadowed name; a NEWTYPE ctor (the
sole variant of its type with arity 1 — such a value is SCALAR-ERASED to its payload, corpus 0292/0598,
not modeled here); and a multi-field ctor (arity ≥ 2, curried payload tuple — not modeled). -/
def variantCtorArity? (m : Module) (name : ByteArray) : Option Nat :=
  if (defNames m).contains name then none
  else match (preludeSumCtors.find? (fun p => name == p.1.toUTF8)).map (·.2) with
  | some ar => some ar
  | none =>
    (userSumTypes m).findSome? (fun (_, ctors) =>
      (ctors.find? (fun c => c.1 == name)).bind (fun c =>
        let isNewtype := ctors.length == 1 && c.2 == 1
        if isNewtype || c.2 ≥ 2 then none else some c.2))

/-- Is `name` a NEWTYPE constructor — the SOLE variant of its user type, carrying EXACTLY ONE field?
Such a sum SCALAR-ERASES: its value IS the payload, construction is identity, a pattern binds the
payload directly (spec type-system.md §"A Single-Variant Single-Field Sum Is A Nominal Type Over Its
Payload", #4516). A multi-variant / nullary / multi-field ctor is NOT a newtype (stays tagged). A
def-shadowed name is not a ctor. -/
def newtypeCtor? (m : Module) (name : ByteArray) : Bool :=
  !((defNames m).contains name) &&
  (userSumTypes m).any (fun (_, ctors) => match ctors with | [(cn, 1)] => cn == name | _ => false)

/-- The constructor NAME an application/pattern head denotes: a bare name head `C`, or a qualified
member-access head `(. T C)` → `C`. -/
def ctorAppName? (m : Module) (children : Array Nat) : Option ByteArray :=
  match children[0]? with
  | some hid =>
    match m.nodes[hid]? with
    | some (Node.atom lid) => match m.leaves[lid]? with | some (Leaf.name b) => some b | _ => none
    | some (Node.list hc) =>
      match m.headName? (Node.list hc) with
      | some dh => if dh == ".".toUTF8 then (hc[2]?).bind (nameOf? m) else none
      | none => none
    | none => none
  | none => none

/-- All top-level `(def (fname param…) body)` statements as (name, (param-spec node ids, body node id)).
Used to resolve a call `(f arg…)` to a user function (a fully-applied call binds each arg to a param). -/
def defTable (m : Module) : List (ByteArray × (Array Nat × Nat)) :=
  match m.nodes[m.root]? with
  | some (Node.list stmts) =>
    stmts.toList.filterMap (fun sid =>
      match asDef? m sid with
      | some dc =>
        match defName? m dc, dc[1]?, dc[dc.size - 1]? with
        | some nm, some targetId, some bodyId => some (nm, (paramSpecNodes m targetId, bodyId))
        | _, _, _ => none
      | none => none)
  | _ => []

/-- A qualified application/value head `(. Q M)` → its (qualifier, member) names. Used to recognize a
prelude MODULE function like `(. Set of)` (a collection builder), distinct from record projection and
from a sum-ctor `(. T C)` (the ctor is dispatched separately by `variantCtorArity?`). -/
def qualHead? (m : Module) (children : Array Nat) : Option (ByteArray × ByteArray) :=
  match children[0]? with
  | some hid =>
    match m.nodes[hid]? with
    | some (Node.list hc) =>
      match m.headName? (Node.list hc) with
      | some dh => if dh == ".".toUTF8 then
                     match (hc[1]?).bind (nameOf? m), (hc[2]?).bind (nameOf? m) with
                     | some q, some mem => some (q, mem)
                     | _, _ => none
                   else none
      | none => none
    | _ => none
  | none => none

/-- Canonicalize a Set's elements: require every element be an orderable scalar (`compareVals` total on
it), SORT by that order, then DEDUPE adjacent equals — the canonical Set form (spec: a Set renders as
`(Set.of (list …sorted-unique))`). `none` if any element is unorderable (a compound/poison) → skip. -/
def canonSet (elems : Array Value) : Option (Array Value) :=
  if elems.all (fun e => (compareVals e e).isSome) then
    let sorted := elems.qsort (fun a b => compareVals a b == some Ordering.lt)
    some (sorted.foldl (fun acc e => if acc.size > 0 && acc[acc.size - 1]! == e then acc else acc.push e) #[])
  else none

/-- Canonicalize a Map's entries: require every KEY be orderable, SORT by key, dedupe by key (a later
entry wins — the canonical Map form is sorted-by-key with unique keys). `none` on an unorderable key. -/
def canonMap (entries : Array (Value × Value)) : Option (Array (Value × Value)) :=
  if entries.all (fun e => (compareVals e.1 e.1).isSome) then
    let sorted := entries.qsort (fun a b => compareVals a.1 b.1 == some Ordering.lt)
    some (sorted.foldl (fun acc e =>
      if acc.size > 0 && acc[acc.size - 1]!.1 == e.1 then acc.set! (acc.size - 1) e else acc.push e) #[])
  else none

/-- `Map.insert m k v`: replace any existing entry for `k`, then add `k ↦ v` (canonicalized by `canonMap`). -/
def mapInsertRaw (entries : Array (Value × Value)) (k v : Value) : Array (Value × Value) :=
  (entries.filter (fun e => !(e.1 == k))).push (k, v)

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
      -- CONSTRUCTION first: a head (bare `C` or qualified `(. T C)`) resolving to a modeled sum
      -- constructor (not shadowed by a local binding). A NEWTYPE ctor ERASES (construction = its
      -- payload); a multi-variant / nullary ctor builds a tagged `variant`.
      let ctorConstruct : Option Outcome :=
        (match (ctorAppName? m children).bind (fun c => if (env.lookup? c).isSome then none else some c) with
         | some cname =>
           if newtypeCtor? m cname then (children[1]?).map (fun pId => evalNode m env defaultIntTy fuel pId)
           else (variantCtorArity? m cname).map (fun ar => evalVariantCtor m env fuel cname ar children)
         | none => none)
        <|> (if qualHead? m children == some ("Set".toUTF8, "of".toUTF8) then some (evalSetOf m env fuel children) else none)
        <|> (if qualHead? m children == some ("Map".toUTF8, "insert".toUTF8) then some (evalMapInsert m env fuel children) else none)
        <|> (match qualHead? m children with            -- QUALIFIED built-in Option/Result ctors → built-in path
             | some (q, c) =>
               if q == "Option".toUTF8 && c == "Some".toUTF8 then some (evalUnaryCtor m env fuel children Value.some)
               else if q == "Option".toUTF8 && c == "None".toUTF8 then some (Outcome.value Value.none)
               else if q == "Result".toUTF8 && c == "Ok".toUTF8 then some (evalUnaryCtor m env fuel children Value.ok)
               else if q == "Result".toUTF8 && c == "Err".toUTF8 then some (evalUnaryCtor m env fuel children Value.err)
               else none
             | none => none)
        <|> ((qualHead? m children).bind (fun (q, f) => evalModuleFn m env fuel q f children))
        <|> ((m.headName? (Node.list children)).bind (fun h =>
               if (env.lookup? h).isSome then none                     -- a local binding shadows: not a top-level call
               else (defTable m).find? (fun d => d.1 == h) |>.bind (fun d =>
                 if d.2.1.size == children.size - 1 then some (evalCall m env fuel d.2.1 d.2.2 children) else none)))
      match ctorConstruct with
      | some o => o
      | none =>
      match m.headName? (Node.list children) with
      | some h =>
        if h == "let".toUTF8 then evalLet m env ty fuel children
        else if h == "if".toUTF8 then evalIf m env ty fuel children
        else if h == ":".toUTF8 then evalAscribe m env ty fuel children
        else if h == "fn".toUTF8 then evalFn m env fuel children
        else if (env.lookup? h).isSome then
          -- the head is a BOUND local — an application of that binding. If it forces to a CLOSURE,
          -- apply it; otherwise (a non-function value applied) it is not modeled → skip.
          match ((env.lookup? h).map (fun e => e.1 ())).getD (.unsupported "") with
          | .value (.closure params body cap) => applyClosure m env fuel params body cap children
          | .value _ => .unsupported "eval: head is a bound non-function value"
          | other => other
        else if h == "Some".toUTF8 then evalUnaryCtor m env fuel children Value.some
        else if h == "Ok".toUTF8 then evalUnaryCtor m env fuel children Value.ok
        else if h == "Err".toUTF8 then evalUnaryCtor m env fuel children Value.err
        else if h == "None".toUTF8 then Outcome.value Value.none
        else if h == "tuple".toUTF8 then evalSeqCtor m env fuel children Value.tuple
        else if h == "list".toUTF8 then evalSeqCtor m env fuel children Value.list
        else if h == "record".toUTF8 then evalRecord m env fuel children
        else if h == ".".toUTF8 then evalProject m env fuel children
        else if h == "match".toUTF8 then evalMatch m env ty fuel children
        else match String.fromUTF8? h with
             | some hs => if arithOps.contains hs then evalArith m env ty fuel hs children
                          else if bitwiseOps.contains hs then evalBitwise m env ty fuel hs children
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
overflow / divide-by-zero per the width. Also handles the UNARY `(- e)` negation (spec: one-operand
subtraction = `0 - e` at the operand's type, so it traps on the MIN-value overflow / an unsigned
underflow exactly as `0 - e` does). -/
partial def evalArith (m : Module) (env : Env) (ty : IntTy) (fuel : Nat) (op : String) (children : Array Nat) : Outcome :=
  if op == "-" && children.size == 2 then
    match children[1]? with
    | some eId =>
      let opTy := ((operandTy? m eId).orElse (fun _ => (nameOf? m eId).bind (fun nm => (env.lookup? nm).bind (·.2)))).getD ty
      match evalNode m env opTy fuel eId with
      | .value (.int a) => evalArithOp "-" 0 a opTy    -- negation = 0 - a at the operand's width
      | .value _ => .unsupported "eval: unary minus of a non-integer"
      | other => other
    | none => .unsupported "eval: malformed unary minus"
  else match children[1]?, children[2]? with
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

/-- `(op a b)` for a binary bitwise / shift operator — same operand evaluation + width inference as
`evalArith` (precedence unsupported > diverges > trap > value), then apply `evalBitOp`. -/
partial def evalBitwise (m : Module) (env : Env) (ty : IntTy) (fuel : Nat) (op : String) (children : Array Nat) : Outcome :=
  match children[1]?, children[2]? with
  | some aId, some bId =>
    if children.size != 3 then .unsupported s!"eval: {op} expects 2 operands"
    else
      let operandTyIn := fun (i : Nat) =>
        (operandTy? m i).orElse (fun _ => (nameOf? m i).bind (fun nm => (env.lookup? nm).bind (·.2)))
      let opTy := ((operandTyIn aId).orElse (fun _ => operandTyIn bId)).getD ty
      let oa := evalNode m env opTy fuel aId
      let ob := evalNode m env opTy fuel bId
      match oa, ob with
      | .unsupported r, _ => .unsupported r
      | _, .unsupported r => .unsupported r
      | .diverges, _ => .diverges
      | _, .diverges => .diverges
      | .trap t, _ => .trap t
      | _, .trap t => .trap t
      | .value (.int a), .value (.int b) => evalBitOp op a b opTy
      | _, _ => .unsupported "eval: non-integer operand to bitwise/shift"
  | _, _ => .unsupported s!"eval: malformed {op}"

/-- A unary constructor `(Ctor e)` (Some/Ok/Err): wrap the payload, storing a non-value payload as a
deferred `poison` (spec Q2: a sum payload defers exactly like a tuple/record field — a
constructed-but-never-observed payload never surfaces its trap). Construction always yields a value. -/
partial def evalUnaryCtor (m : Module) (env : Env) (fuel : Nat) (children : Array Nat)
    (wrap : Value → Value) : Outcome :=
  match children[1]? with
  | some eId => .value (wrap (outcomeToValue (evalNode m env defaultIntTy fuel eId)))
  | none => .unsupported "eval: malformed unary constructor"

/-- `(Set.of (list e…))` = `((. Set of) (list e…))` — evaluate the list argument, then canonicalize its
elements (sort + dedupe) into a Set value. A non-list arg or an unorderable element → skip. -/
partial def evalSetOf (m : Module) (env : Env) (fuel : Nat) (children : Array Nat) : Outcome :=
  match children[1]? with
  | some listId =>
    match evalNode m env defaultIntTy fuel listId with
    | .value (.list elems) => match canonSet elems with
                              | some s => .value (.set s)
                              | none => .unsupported "eval: Set with an unorderable/unobserved element"
    | .value _ => .unsupported "eval: Set.of argument is not a list"
    | other => other
  | none => .unsupported "eval: malformed Set.of"

/-- `Map.insert m k v` = `((. Map insert) m k v)` (flat 3-arg) — insert/replace `k ↦ v` in map `m`,
canonicalized (sorted by key, unique keys). A non-map operand or an unorderable key → skip. -/
partial def evalMapInsert (m : Module) (env : Env) (fuel : Nat) (children : Array Nat) : Outcome :=
  match children[1]?, children[2]?, children[3]? with
  | some mId, some kId, some vId =>
    match evalNode m env defaultIntTy fuel mId with
    | .value (.map entries) =>
      match evalNode m env defaultIntTy fuel kId with
      | .value k =>
        match evalNode m env defaultIntTy fuel vId with
        | .value v => match canonMap (mapInsertRaw entries k v) with
                      | some cm => .value (.map cm)
                      | none => .unsupported "eval: Map with an unorderable key"
        | other => other
      | other => other
    | .value _ => .unsupported "eval: Map.insert on a non-map"
    | other => other
  | _, _, _ => .unsupported "eval: malformed Map.insert"

/-- A fully-applied call `(f arg…)` of a top-level `def (f param…)`: bind each arg LAZILY (a thunk over
the CALLER's env, so an unused parameter's arg is never forced — spec) under its parameter name +
declared integer type, then evaluate the body in that fresh scope (top-level defs/ctors resolve globally,
not via env; recursion is fuel-bounded). Partial application / first-class `fn` closures are NOT modeled
here (they never reach this — a partial call has a wrong arg count, a closure head is a bound local). -/
partial def evalCall (m : Module) (env : Env) (fuel : Nat) (paramSpecs : Array Nat) (bodyId : Nat) (children : Array Nat) : Outcome :=
  let args := children.extract 1 children.size
  let bindings := (paramSpecs.zip args).filterMap (fun (specId, argId) =>
    (paramSpec? m specId).map (fun (nm, ty) => (nm, (fun _ => evalNode m env defaultIntTy fuel argId), ty)))
  if bindings.size == paramSpecs.size then evalNode m bindings.toList defaultIntTy fuel bodyId
  else .unsupported "eval: call has a malformed parameter spec"

/-- `(fn (param…) body)` → a closure value capturing the CURRENT env (each binding forced now to a value
or a `poison`, so an unused captured binding never surfaces its trap; laziness preserved via poison). -/
partial def evalFn (m : Module) (env : Env) (fuel : Nat) (children : Array Nat) : Outcome :=
  match children[1]?, children[2]? with
  | some paramListId, some bodyId =>
    let params := match m.nodes[paramListId]? with | some (Node.list ps) => ps | _ => #[]
    let cap := env.map (fun e => (e.1, outcomeToValue (e.2.1 ())))
    .value (.closure params bodyId cap)
  | _, _ => .unsupported "eval: malformed fn"

/-- Apply a closure to a FULLY-supplied argument list: bind each arg LAZILY (over the caller's env) under
its parameter name + declared type, plus the captured env (each name → its stored value, observed
shallowly on use), then evaluate the body. A partial application (wrong arg count) is not modeled → skip. -/
partial def applyClosure (m : Module) (env : Env) (fuel : Nat) (params : Array Nat) (body : Nat)
    (cap : List (ByteArray × Value)) (children : Array Nat) : Outcome :=
  let args := children.extract 1 children.size
  if params.size != args.size then .unsupported "eval: closure arity mismatch (partial application not modeled)"
  else
    let argBindings : Env := (params.zip args).toList.filterMap (fun (specId, argId) =>
      (paramSpec? m specId).map (fun (nm, ty) => (nm, (fun _ => evalNode m env defaultIntTy fuel argId), ty)))
    let capBindings : Env := cap.map (fun (nm, v) => (nm, (fun _ => observeShallow v), Option.none))
    if argBindings.length == params.size then evalNode m (argBindings ++ capBindings) defaultIntTy fuel body
    else .unsupported "eval: closure has a malformed parameter spec"

/-- Function-FREE collection query/update module fns (flat `((. Mod fn) args…)`): `List.len`,
`List.concat`, `Set.contains`, `Map.len`, `Map.lookup` (→ Option), `Map.remove`. `none` = not one of
these (fall through). Ops taking a FUNCTION arg (map/filter/fold) or Option-returning index (`.at`/`.get`)
are not handled here. -/
partial def evalModuleFn (m : Module) (env : Env) (fuel : Nat) (qual mem : ByteArray) (children : Array Nat) : Option Outcome :=
  let a1 := (children[1]?).map (fun i => evalNode m env defaultIntTy fuel i)
  let a2 := (children[2]?).map (fun i => evalNode m env defaultIntTy fuel i)
  let is := fun (q f : String) => qual == q.toUTF8 && mem == f.toUTF8
  if is "List" "len" then
    some (match a1 with | some (.value (.list es)) => .value (.int es.size)
                        | some (.value _) => .unsupported "List.len: not a list" | some o => o | none => .unsupported "List.len arity")
  else if is "Map" "len" then
    some (match a1 with | some (.value (.map es)) => .value (.int es.size)
                        | some (.value _) => .unsupported "Map.len: not a map" | some o => o | none => .unsupported "Map.len arity")
  else if is "List" "push" then
    -- append an element (deferred as a poison if non-value, like a list literal element).
    some (match a1 with
          | some (.value (.list es)) =>
            (match children[2]? with
             | some xId => .value (.list (es.push (outcomeToValue (evalNode m env defaultIntTy fuel xId))))
             | none => .unsupported "List.push arity")
          | some (.value _) => .unsupported "List.push: not a list" | some o => o | none => .unsupported "List.push arity")
  else if is "Bytes" "concat" then
    some (match a1, a2 with
          | some (.value (.bytes x)), some (.value (.bytes y)) => .value (.bytes (x ++ y))
          | some (.unsupported r), _ | _, some (.unsupported r) => .unsupported r
          | some (.trap t), _ | _, some (.trap t) => .trap t
          | some .diverges, _ | _, some .diverges => .diverges
          | _, _ => .unsupported "Bytes.concat: non-bytes operand")
  else if is "Option" "expect" then
    -- unwrap `Some x` → x (observed); `None` traps with the given message (a custom trap → not modeled → skip).
    some (match a1 with
          | some (.value (.some x)) => observeShallow x
          | some (.value .none) => .unsupported "Option.expect on None (trap-message semantics not modeled)"
          | some (.value _) => .unsupported "Option.expect: operand is not an Option" | some o => o | none => .unsupported "Option.expect arity")
  else if is "List" "concat" then
    some (match a1, a2 with
          | some (.value (.list x)), some (.value (.list y)) => .value (.list (x ++ y))
          | some (.unsupported r), _ | _, some (.unsupported r) => .unsupported r
          | some (.trap t), _ | _, some (.trap t) => .trap t
          | some .diverges, _ | _, some .diverges => .diverges
          | _, _ => .unsupported "List.concat: non-list operand")
  else if is "Set" "contains" then
    some (match a1, a2 with
          | some (.value (.set es)), some (.value x) => .value (.bool (es.any (· == x)))
          | some (.unsupported r), _ | _, some (.unsupported r) => .unsupported r
          | some (.trap t), _ | _, some (.trap t) => .trap t
          | some .diverges, _ | _, some .diverges => .diverges
          | _, _ => .unsupported "Set.contains: operand")
  else if is "Map" "lookup" then
    some (match a1, a2 with
          | some (.value (.map es)), some (.value k) =>
            (match (es.find? (fun kv => kv.1 == k)).map (·.2) with | some v => .value (.some v) | none => .value .none)
          | some (.unsupported r), _ | _, some (.unsupported r) => .unsupported r
          | some (.trap t), _ | _, some (.trap t) => .trap t
          | some .diverges, _ | _, some .diverges => .diverges
          | _, _ => .unsupported "Map.lookup: operand")
  else if is "Bytes" "of" then
    -- `Bytes.of (list i…)` — build a Bytes value from a list of byte-valued ints (0..255).
    some (match a1 with
          | some (.value (.list es)) =>
            if es.all (fun e => match e with | .int n => 0 ≤ n && n < 256 | _ => false) then
              .value (.bytes (ByteArray.mk (es.map (fun e => match e with | .int n => UInt8.ofNat n.toNat | _ => 0))))
            else .unsupported "Bytes.of: element is not a 0..255 byte"
          | some (.value _) => .unsupported "Bytes.of: not a list" | some o => o | none => .unsupported "Bytes.of arity")
  else if is "String" "at" then
    -- indexed CHARACTER access (by Unicode SCALAR, matching Lean's String.data) → Option single-char
    -- String: `Some s[i]` when 0 ≤ i < char-count, else `None`. (`"café"[3]="é"`, `"😀b"[1]="b"`.)
    some (match a1, a2 with
          | some (.value (.str bytes)), some (.value (.int i)) =>
            (match String.fromUTF8? bytes with
             | some s => let cs := s.data
                         if 0 ≤ i && i < Int.ofNat cs.length then .value (.some (.str (String.toUTF8 (cs[i.toNat]!).toString)))
                         else .value .none
             | none => .unsupported "String.at: invalid UTF-8")
          | some (.unsupported r), _ | _, some (.unsupported r) => .unsupported r
          | some (.trap t), _ | _, some (.trap t) => .trap t
          | some .diverges, _ | _, some .diverges => .diverges
          | _, _ => .unsupported "String.at: operand")
  else if is "Bytes" "at" then
    -- indexed BYTE access → Option Int: `Some b[i]` when 0 ≤ i < len, else `None`.
    some (match a1, a2 with
          | some (.value (.bytes b)), some (.value (.int i)) =>
            if 0 ≤ i && i < Int.ofNat b.size then .value (.some (.int (Int.ofNat (b[i.toNat]!).toNat))) else .value .none
          | some (.unsupported r), _ | _, some (.unsupported r) => .unsupported r
          | some (.trap t), _ | _, some (.trap t) => .trap t
          | some .diverges, _ | _, some .diverges => .diverges
          | _, _ => .unsupported "Bytes.at: operand")
  else if (is "List" "at" || is "List" "get") then
    -- indexed access → Option: `Some l[i]` when 0 ≤ i < len, else `None` (out-of-bounds / negative).
    some (match a1, a2 with
          | some (.value (.list es)), some (.value (.int i)) =>
            if 0 ≤ i && i < Int.ofNat es.size then .value (.some (es[i.toNat]!)) else .value .none
          | some (.unsupported r), _ | _, some (.unsupported r) => .unsupported r
          | some (.trap t), _ | _, some (.trap t) => .trap t
          | some .diverges, _ | _, some .diverges => .diverges
          | _, _ => .unsupported "List.at: operand")
  else if is "Map" "remove" then
    some (match a1, a2 with
          | some (.value (.map es)), some (.value k) =>
            (match canonMap (es.filter (fun kv => !(kv.1 == k))) with | some cm => .value (.map cm) | none => .unsupported "Map.remove: unorderable key")
          | some (.unsupported r), _ | _, some (.unsupported r) => .unsupported r
          | some (.trap t), _ | _, some (.trap t) => .trap t
          | some .diverges, _ | _, some .diverges => .diverges
          | _, _ => .unsupported "Map.remove: operand")
  else none

/-- A generic sum constructor application `(C …)` / `((. T C) …)`: nullary → `variant C unit`; single-field
→ `variant C payload` (payload deferred as a `poison` if non-value, like a tuple/record field — spec Q2). -/
partial def evalVariantCtor (m : Module) (env : Env) (fuel : Nat) (cname : ByteArray) (arity : Nat) (children : Array Nat) : Outcome :=
  match arity with
  | 0 => .value (.variant cname .unit)
  | _ => match children[1]? with
         | some pId => .value (.variant cname (outcomeToValue (evalNode m env defaultIntTy fuel pId)))
         | none => .value (.variant cname .unit)

/-- A sequence constructor `(tuple e…)` / `(list e…)`: evaluate each element, storing a non-value
element as a `poison` (deferred) rather than propagating it — an element that is never observed
(projected, or flowed to the result) never surfaces its trap. Construction itself always yields a
value. -/
partial def evalSeqCtor (m : Module) (env : Env) (fuel : Nat) (children : Array Nat)
    (wrap : Array Value → Value) : Outcome :=
  let rec go (js : List Nat) (acc : Array Value) : Array Value :=
    match js with
    | [] => acc
    | j :: rest => go rest (acc.push (outcomeToValue (evalNode m env defaultIntTy fuel j)))
  .value (wrap (go (children.extract 1 children.size).toList #[]))

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
  | .value va, .value vb =>
    -- comparing INSPECTS both operands fully → observe deeply so a deferred poison (a trapping compound
    -- element) surfaces its trap rather than being compared as data (spec observation rule).
    match observeDeep va with
    | .value fa => match observeDeep vb with
                   | .value fb => k fa fb
                   | other => other
    | other => other

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

/-- A record-construction field entry `(= key val)` (equality-shaped) or `(key val)` (bare) → its key
name + value node id. -/
partial def recordField? (m : Module) (fid : Nat) : Option (ByteArray × Nat) :=
  match m.nodes[fid]? with
  | some (Node.list fc) =>
    match m.headName? (Node.list fc) with
    | some h =>
      if h == "=".toUTF8 && fc.size == 3 then (nameOf? m fc[1]!).map (fun k => (k, fc[2]!))
      else if fc.size == 2 then (nameOf? m fc[0]!).map (fun k => (k, fc[1]!))
      else none
    | none => none
  | _ => none

/-- `(record (= k v)… )` / `(record (k v)… )` — evaluate each field's value (a non-value stored as a
deferred `poison`, like a tuple element), collect keyed fields, then SORT by key so the record's
canonical form is order-insensitive (deterministic-value-form.md: a record's canonical byte form sorts
its fields by key) and structural `BEq` compares by field SET. -/
partial def evalRecord (m : Module) (env : Env) (fuel : Nat) (children : Array Nat) : Outcome :=
  let rec go (js : List Nat) (acc : Array (ByteArray × Value)) : Except String (Array (ByteArray × Value)) :=
    match js with
    | [] => .ok acc
    | j :: rest =>
      match recordField? m j with
      | some (k, vId) => go rest (acc.push (k, outcomeToValue (evalNode m env defaultIntTy fuel vId)))
      | none => .error "eval: malformed record field"
  match go (children.extract 1 children.size).toList #[] with
  | .ok fields => .value (.record (fields.qsort (fun a b => cmpBytes a.1 b.1 == .lt)))
  | .error e => .unsupported e

/-- `(. recExpr field)` — project a named field from a record value (spec §Member Access): observe the
projected field SHALLOWLY (its top-level poison surfaces; a nested compound stays lazy). A non-record
operand or a non-name field key (tuple positional access, etc.) is not modeled → skip. -/
partial def evalProject (m : Module) (env : Env) (fuel : Nat) (children : Array Nat) : Outcome :=
  match children[1]?, children[2]? with
  | some recId, some fieldId =>
    -- `(. Map empty)` used as a value = the empty map (a prelude module value, not a record projection).
    if (nameOf? m recId == some "Map".toUTF8) && (nameOf? m fieldId == some "empty".toUTF8) then .value (.map #[])
    else
    match evalNode m env defaultIntTy fuel recId with
    | .value (.record fields) =>
      match nameOf? m fieldId with
      | some key =>
        match (fields.find? (fun kv => kv.1 == key)).map (·.2) with
        | some fv => observeShallow fv
        | none => .unsupported "eval: record has no such field (typecheck not modeled)"
      | none => .unsupported "eval: projection key is not a field name"
    | .value _ => .unsupported "eval: projection operand is not a record (tuple/other access not modeled)"
    | other => other
  | _, _ => .unsupported "eval: malformed projection"

/-- Match a pattern against an ALREADY-FORCED subject value — a purely structural test that binds names
LAZILY and yields the environment extension on a match. `.error o` = decided (a forced non-value from a
literal comparison, or an `unsupported` for a pattern shape we do not model — aborts the arm with `o`);
`.ok none` = no match (try the next arm); `.ok (some ext)` = matched, add bindings `ext`. Modeled:
wildcard `_`, bare-name binder (binds `subj` LAZILY, so an unused binder never forces it — spec Q2),
scalar literal (forces + compares), and the decomposition patterns `(Some p)`/`(Ok p)`/`(Err p)`/
`(None _)`/`(tuple p…)`/`(record (= k p)…)` recursively. A user-sum or other head → `unsupported`. -/
partial def matchPat (m : Module) (patId : Nat) (subj : Value) : Except Outcome (Option Env) :=
  match m.nodes[patId]? with
  | some (Node.atom lid) =>
    match m.leaves[lid]? with
    | some (Leaf.name b) =>
      if b == "_".toUTF8 then .ok (some [])
      else .ok (some [(b, (fun _ => observeShallow subj), Option.none)])
    | some pl =>
      match Value.ofLeaf pl with
      | some litV => match observeDeep subj with
                     | .value fp => if fp == litV then .ok (some []) else .ok none
                     | other => .error other
      | none => .error (.unsupported "eval: match literal pattern is a non-scalar leaf")
    | none => .error (.unsupported "eval: match pattern leaf out of range")
  | some (Node.list pc) =>
    -- a sum constructor pattern. A NEWTYPE pattern `(Mk subpat)` binds the payload DIRECTLY (the
    -- scrutinee IS the erased payload); a tagged-variant pattern `(C subpat)` / `((. T C) subpat)` /
    -- nullary `(C)` compares the tag then binds the payload (multi-field/shadowed heads are not modeled).
    let ctorMatch : Option (Except Outcome (Option Env)) :=
      match ctorAppName? m pc with
      | some cname =>
        if newtypeCtor? m cname then some (match pc[1]? with | some sp => matchPat m sp subj | none => .ok (some []))
        else if (variantCtorArity? m cname).isSome then
          some (match subj with
                | .variant tag payload => if tag == cname then (match pc[1]? with | some sp => matchPat m sp payload | none => .ok (some [])) else .ok none
                | _ => .ok none)
        else none
      | none => none
    match ctorMatch with
    | some r => r
    | none =>
    match m.headName? (Node.list pc) with
    | some ph =>
      if ph == "Some".toUTF8 then (match subj, pc[1]? with | .some p, some sp => matchPat m sp p | .some _, none => .error (.unsupported "eval: malformed Some pattern") | _, _ => .ok none)
      else if ph == "Ok".toUTF8 then (match subj, pc[1]? with | .ok p, some sp => matchPat m sp p | .ok _, none => .error (.unsupported "eval: malformed Ok pattern") | _, _ => .ok none)
      else if ph == "Err".toUTF8 then (match subj, pc[1]? with | .err p, some sp => matchPat m sp p | .err _, none => .error (.unsupported "eval: malformed Err pattern") | _, _ => .ok none)
      else if ph == "None".toUTF8 then (match subj, pc[1]? with | .none, some sp => matchPat m sp .unit | .none, none => .ok (some []) | _, _ => .ok none)
      else if ph == "tuple".toUTF8 then
        (match subj with
         | .tuple es => let sps := pc.extract 1 pc.size
                        if sps.size != es.size then .ok none else matchSeq m (sps.zip es).toList
         | _ => .ok none)
      else if ph == "record".toUTF8 then
        (match subj with
         | .record fields => matchRecordPats m (pc.extract 1 pc.size).toList fields
         | _ => .ok none)
      else .error (.unsupported "eval: match user-sum/other constructor pattern not modeled")
    | none => .error (.unsupported "eval: match pattern is a headless list")
  | none => .error (.unsupported "eval: match pattern node out of range")

/-- Match `(pattern, subject)` pairs left-to-right, ANDing results and concatenating bindings; a
no-match or a decided outcome short-circuits. -/
partial def matchSeq (m : Module) (pairs : List (Nat × Value)) : Except Outcome (Option Env) :=
  match pairs with
  | [] => .ok (some [])
  | (pid, sv) :: rest =>
    match matchPat m pid sv with
    | .error o => .error o
    | .ok none => .ok none
    | .ok (some e1) => match matchSeq m rest with
                       | .error o => .error o
                       | .ok none => .ok none
                       | .ok (some e2) => .ok (some (e1 ++ e2))

/-- Match record field-patterns `(= k p)…` against a record's fields: each named key MUST be present
(else no match), its value matched by the field's sub-pattern. -/
partial def matchRecordPats (m : Module) (fieldPats : List Nat) (fields : Array (ByteArray × Value)) : Except Outcome (Option Env) :=
  match fieldPats with
  | [] => .ok (some [])
  | fp :: rest =>
    match recordField? m fp with
    | some (key, subPatId) =>
      match (fields.find? (fun kv => kv.1 == key)).map (·.2) with
      | some fv =>
        match matchPat m subPatId fv with
        | .error o => .error o
        | .ok none => .ok none
        | .ok (some e1) => match matchRecordPats m rest fields with
                           | .error o => .error o
                           | .ok none => .ok none
                           | .ok (some e2) => .ok (some (e1 ++ e2))
      | none => .ok none   -- named key absent → no match
    | none => .error (.unsupported "eval: malformed record field-pattern")

/-- `(match scrutinee (pat body)… )` — try arms in order (spec: the scrutinee IS an observation point,
core-semantics.md:287). A top-level WILDCARD `_` or bare-name BINDER binds the scrutinee LAZILY (never
forcing it, like a `let`); every other pattern (scalar literal, and the `(Some p)`/`(Ok p)`/`(Err p)`/
`(None _)`/`(tuple p…)`/`(record (= k p)…)` decomposition patterns via `matchPat`) forces the scrutinee
(observed) and matches structurally, binding sub-values LAZILY. A user-sum or unmodeled pattern → the
whole match is `unsupported` (a sound skip — arm selection cannot be soundly decided past it). -/
partial def evalMatch (m : Module) (env : Env) (ty : IntTy) (fuel : Nat) (children : Array Nat) : Outcome :=
  match children[1]? with
  | none => .unsupported "eval: malformed match (no scrutinee)"
  | some scrutId =>
    -- `none` = arm did not match, try the next; `some o` = this arm decided the match.
    let matchArm := fun (patId bodyId : Nat) =>
      -- the forced path: observe the scrutinee, then match structurally via `matchPat`
      let forced := fun (_ : Unit) =>
        match evalNode m env defaultIntTy fuel scrutId with
        | .value sv0 =>
          match observeShallow sv0 with
          | .value sv => match matchPat m patId sv with
                         | .error o => some o
                         | .ok none => none
                         | .ok (some ext) => some (evalNode m (ext ++ env) ty fuel bodyId)
          | other => some other
        | other => some other
      match m.nodes[patId]? with
      | some (Node.atom lid) =>
        match m.leaves[lid]? with
        | some (Leaf.name b) =>
          if b == "_".toUTF8 then some (evalNode m env ty fuel bodyId)          -- wildcard: no force
          else some (evalNode m ((b, (fun _ => evalNode m env defaultIntTy fuel scrutId), Option.none) :: env)
                       ty fuel bodyId)                                          -- binder: lazy bind
        | _ => forced ()                                                        -- scalar literal
      | _ => forced ()                                                          -- decomposition pattern
    let rec tryArms (arms : List Nat) : Outcome :=
      match arms with
      | [] => .unsupported "eval: match fell through (non-exhaustive / no modeled arm matched)"
      | armId :: rest =>
        match m.nodes[armId]? with
        | some (Node.list ac) =>
          match ac[0]?, ac[1]? with
          | some patId, some bodyId =>
            match matchArm patId bodyId with
            | some o => o
            | none => tryArms rest
          | _, _ => .unsupported "eval: malformed match arm"
        | _ => .unsupported "eval: match arm is not a list"
    tryArms (children.extract 2 children.size).toList
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
        -- the result flows to the program output → observe it DEEPLY, so a deferred poison (a trapping
        -- element of a returned compound) surfaces its trap at the output boundary.
        match Eval.evalNode m bindings.toList Eval.defaultIntTy Eval.defaultFuel bodyId with
        | .value v => Eval.observeDeep v
        | other => other
  | none => .unsupported "execute: program is not a (do (def (main …) BODY) (export main)) form"

/-- STAGE 1 — const-evaluate a closed program to its minimal form (grades a bare `(input E)` case).
Equal to `execute` with no arguments (stage parity holds by construction). -/
def reduce (m : Ast.Module) : Outcome := execute m #[]

end Oracle
