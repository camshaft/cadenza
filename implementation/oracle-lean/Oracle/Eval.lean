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
  -- a `?`/`try` short-circuit in flight: the enclosing fallible function must abruptly RETURN this
  -- value (an `Err e` / `None`). Propagates like a trap through every combinator until it reaches the
  -- FUNCTION BOUNDARY (evalCall / applyClosure / execute), where it becomes the function's `.value`.
  | errReturn (v : Value)
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

/-- The param-spec nodes + body of the top-level `(def (<target> param…) BODY)` named `target`. Used to
run a trial that calls a NAMED export — a program may export several defs (`(export same)` `(export
implied)` …) with NO `main`, and a trial's `(call <export> …)` names which one to run. -/
def namedParamsBody? (m : Module) (target : ByteArray) : Option (Array Nat × Nat) := do
  let root ← m.nodes[m.root]?
  match root with
  | Node.list stmts =>
    stmts.toList.findSome? (fun sid =>
      match asDef? m sid with
      | some dc =>
        match defName? m dc, dc[1]?, dc[dc.size - 1]? with
        | some nm, some targetId, some bodyId =>
          if nm == target then some (paramSpecNodes m targetId, bodyId) else none
        | _, _, _ => none
      | none => none)
  | _ => none

def mainParamsBody? (m : Module) : Option (Array Nat × Nat) := namedParamsBody? m "main".toUTF8

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

/-- A lazily-computed binding outcome. Uses Lean's built-in `Thunk` so forcing is MEMOIZED — evaluated
at most once and cached. This is load-bearing for RECURSION: without memoization, a recursive binding
chain (e.g. a tail-recursive accumulator's `i`/`acc` re-derived each iteration) re-forces the whole
chain on every access → O(n²) blow-up that effectively HANGS on a large loop. Memoized → O(n). -/
abbrev Thunk := _root_.Thunk Outcome

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

/-- Parse an integer TYPE NAME (`Int8`/`UInt64`/`BigInt`/…) to an `IntTy` — the name-leaf case, for
recognizing a numeric-conversion module qualifier like `UInt8` in `(. UInt8 wrap)`. -/
def parseIntTyName? (b : ByteArray) : Option IntTy :=
  match String.fromUTF8? b with
  | some "BigInt" => some { signed := true, width := .big }
  | some s =>
    if s.startsWith "Int" then (s.drop 3).toNat?.map (fun w => { signed := true, width := .bits w })
    else if s.startsWith "UInt" then (s.drop 4).toNat?.map (fun w => { signed := false, width := .bits w })
    else none
  | none => none

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

/-- The overflow mode for unqualified `+`/`-`/`*` of the given signedness, from a module-level
`(pragma overflow (signed <mode>) (unsigned <mode>))` directive (corpus 06-numeric). Returns `true`
(WRAP: two's-complement mod 2^w) when the pragma sets THIS signedness to `wrap`; otherwise `false`
(the default trap-on-overflow). Signedness-selective: `(signed wrap)` governs only signed ops, so an
unsigned overflow under a signed-only wrap pragma still traps. The runtime codegen reads the SAME
`overflow_mode_of`, so a wrap module's constant and runtime overflow agree — the oracle must too, else
it emits a false `trap overflow` where the program yields a wrapped value. -/
def overflowWraps? (m : Module) (signed : Bool) : Bool :=
  let wantSign : ByteArray := (if signed then "signed" else "unsigned").toUTF8
  m.nodes.any (fun node =>
    match node with
    | Node.list cs =>
      headName? m node == some "pragma".toUTF8 &&
      (cs[1]?).bind (nameOf? m) == some "overflow".toUTF8 &&
      -- among the `(signedness mode)` pairs (children from index 2 on), OUR signedness is set to `wrap`
      (cs.extract 2 cs.size).any (fun cid =>
        match m.nodes[cid]? with
        | some (Node.list scs) =>
          (scs[0]?).bind (nameOf? m) == some wantSign && (scs[1]?).bind (nameOf? m) == some "wrap".toUTF8
        | _ => false)
    | _ => false)

/-- Evaluate a binary integer operator, trapping on overflow / divide-by-zero per `ty`. Division and
remainder truncate toward zero (matching the checked wasm `i64.div_s`/`rem_s` the compiler emits). An
`unknown` width makes overflow undecidable → `unsupported` (a sound coverage-gap, never a guess);
`big` never overflows. `wrapOverflow` (from a `(pragma overflow … wrap)` module, via `overflowWraps?`)
makes `+`, `-`, `*` overflow WRAP (two's-complement mod 2^w) instead of trapping — division,
remainder, and the signed MIN-over-minus-one case are unaffected (the pragma governs only `+`, `-`, `*`). -/
def evalArithOp (op : String) (a b : Int) (ty : IntTy) (wrapOverflow : Bool := false) : Outcome :=
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
      if inB r then .value (.int r)
      else if wrapOverflow then
        -- two's-complement wrap mod 2^w (SAME formula as `(. <IntTy> wrapping-add) …`)
        let modw : Int := (2 : Int) ^ w
        let p := ((r % modw) + modw) % modw
        .value (.int (if ty.signed && p ≥ (2 : Int) ^ (w - 1) then p - modw else p))
      else .trap "overflow"

/-- Binary FLOAT arithmetic on the operands' `f64` values. A float arith result is a COMPUTED `f64`
(`Value.f64`), compared bit-exact by `valueEqSpec` against any float spelling; IEEE division by zero
yields ±inf / NaN (never a trap, unlike integer `/`). `%` (modulo) is not modeled for floats. -/
def evalFloatOp (op : String) (a b : Float) : Outcome :=
  if op == "+" then .value (.f64 (a + b))
  else if op == "-" then .value (.f64 (a - b))
  else if op == "*" then .value (.f64 (a * b))
  else if op == "/" then .value (.f64 (a / b))
  else .unsupported "eval: float % (modulo) not modeled"

/-- Does the subtree at `i` mention a `Float32` type anywhere (a `(: e Float32)` ascription)? A SAFE
over-approximation used to SKIP float arithmetic that involves Float32: exact f32 arith rounds at each op
(the evaluator doesn't yet thread float precision), so grading it at f64 would be wrong. Float64 arith
(no Float32 mention) is unaffected and grades normally. Float32 arithmetic is a pending increment. -/
partial def mentionsFloat32? (m : Module) (i : Nat) : Bool :=
  match m.nodes[i]? with
  | some (Node.list cs) =>
    (m.headName? (Node.list cs) == some ":".toUTF8 && (cs[2]?).bind (nameOf? m) == some "Float32".toUTF8)
    || cs.any (mentionsFloat32? m)
  | _ => false

/-- Does the subtree at `i` contain a `(try …)` whose boundary is THIS function (a `?` that would
short-circuit here)? Used to make a `let` binding whose value contains a `?` EAGER: the `?` short-circuit
is control flow that fires when the binding is reached, even if the bound name is never forced (the oracle
otherwise binds LAZILY). Does NOT descend into a nested `(fn …)` — an inner closure's `?` binds to that
closure's boundary, not this one (and building the closure runs no `?`). -/
partial def mentionsTry? (m : Module) (i : Nat) : Bool :=
  match m.nodes[i]? with
  | some (Node.list cs) =>
    match m.headName? (Node.list cs) with
    | some h => if h == "try".toUTF8 then true
                else if h == "fn".toUTF8 then false
                else cs.any (mentionsTry? m)
    | none => cs.any (mentionsTry? m)
  | _ => false

/-- Is the expression at `i` (through an ascription) a heap-collection CONSTRUCTION — a `(list …)` /
`(set …)` / `(map …)` literal or a `Set.of` / `Map.insert`? Such a construction is STRICT in its element
arguments (operator ruling A, #5194/#5332): the args are forced whenever it is reached, EVEN when the
result is bound-and-discarded — so a `let` binding to one must be evaluated EAGERLY (a trapping arg traps
though the collection is never observed), not deferred as a lazy thunk. -/
partial def rhsIsStrictCtor? (m : Module) (i : Nat) : Bool :=
  match m.nodes[i]? with
  | some (Node.list cs) =>
    match m.headName? (Node.list cs) with
    | some h =>
      if h == ":".toUTF8 && cs.size ≥ 2 then (match cs[1]? with | some j => rhsIsStrictCtor? m j | none => false)
      else h == "list".toUTF8 || h == "set".toUTF8 || h == "map".toUTF8
    | none =>
      -- a member-headed construction `((. Set of) …)` / `((. Map insert) …)` (qualHead? is defined later)
      match (cs[0]?).bind (fun hid => m.nodes[hid]?) with
      | some (Node.list hc) =>
        if m.headName? (Node.list hc) == some ".".toUTF8 then
          match (hc[1]?).bind (nameOf? m), (hc[2]?).bind (nameOf? m) with
          | some q, some mem => (q == "Set".toUTF8 && mem == "of".toUTF8) || (q == "Map".toUTF8 && mem == "insert".toUTF8)
          | _, _ => false
        else false
      | _ => false
  | _ => false

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
  -- exact Rational order: a/b < c/d ⟺ a·d < c·b (both denominators normalized POSITIVE).
  | .rational a b, .rational c d => some (compare (a * d) (c * b))
  | _, _ => none

/-- Whether a relational operator holds given the three-way `Ordering` of its operands. -/
def cmpHolds (op : String) : Ordering → Bool
  | o => match op with
         | "<" => o == .lt
         | ">" => o == .gt
         | "<=" => o != .gt
         | ">=" => o != .lt
         | _ => false

/-- A distinct rank per `Value` constructor, so two DIFFERENT (non-float) constructors get a total order. -/
def valRank : Value → Nat
  | .int _ => 0 | .bool _ => 1 | .str _ => 2 | .char _ => 3 | .bytes _ => 4
  | .float .. => 5 | .floatNan => 6 | .floatInf _ => 7 | .f64 _ => 8 | .unit => 9
  | .some _ => 10 | .none => 11 | .ok _ => 12 | .err _ => 13 | .tuple _ => 14
  | .list _ => 15 | .record _ => 16 | .set _ => 17 | .map _ => 18 | .variant .. => 19
  | .closure .. => 20 | .poison _ => 21 | .rational .. => 22

mutual
/-- A total STRUCTURAL order over NON-FLOAT values, for CANONICALIZING a set/map with COMPOUND keys/
elements. A set/map value is order-INSENSITIVE, so any CONSISTENT total order canonicalizes it (sort +
dedupe); this recursive lexicographic order is consistent with structural `==`. `none` if a FLOAT (no total
order threaded here — float-form-mixing / NaN) or a `closure`/`poison` is encountered → that key/element
stays "unorderable" (a sound skip, as before). Different non-float constructors are ordered by `valRank`. -/
partial def cmpValue (a b : Value) : Option Ordering :=
  -- BOTH floats (any form: `.float`/`.f64`/`.floatNan`/`.floatInf`) → a total order by CANONICAL f64 bits,
  -- so the float FORMS unify (`.float 1.5` == `.f64 1.5`) and all NaN collapse to one key (spec: a single
  -- NaN). `-0.0`/`+0.0` keep distinct bits (spec: sign-significant zero). For CANONICALIZATION, not IEEE.
  match Value.asF64? a, Value.asF64? b with
  | some fa, some fb =>
    -- canonical NaN key = the standard quiet-NaN bits `0x7ff8000000000000` (a NaN pattern, so distinct
    -- from EVERY finite/±inf), NOT 0 — `(0.0).toBits` IS 0, which would collide NaN with 0.0 (a set
    -- `#set(NaN 0.0)` must have len 2). All NaN spellings map to it → NaN==NaN dedupes.
    let key := fun (f : Float) => if f.isNaN then (0x7ff8000000000000 : UInt64) else f.toBits
    some (compare (key fa) (key fb))
  | _, _ =>
  match a, b with
  | .int x, .int y => some (compare x y)
  | .rational a b, .rational c d => some (compare (a * d) (c * b))   -- exact rational order (dens positive)
  | .bool x, .bool y => some (compare (x == true) (y == true))
  | .str x, .str y => some (cmpBytes x y)
  | .char x, .char y => some (cmpBytes x y)
  | .bytes x, .bytes y => some (cmpBytes x y)
  | .unit, .unit => some .eq
  | .none, .none => some .eq
  | .some x, .some y => cmpValue x y
  | .ok x, .ok y => cmpValue x y
  | .err x, .err y => cmpValue x y
  | .tuple xs, .tuple ys => cmpValSeq xs ys 0
  | .list xs, .list ys => cmpValSeq xs ys 0
  | .set xs, .set ys => cmpValSeq xs ys 0
  | .variant t1 p1, .variant t2 p2 => (match cmpBytes t1 t2 with | .eq => cmpValue p1 p2 | o => some o)
  | .record fs, .record gs => cmpValFields fs gs 0
  | .map xs, .map ys => cmpValMapEntries xs ys 0
  | .float .., _ | .f64 _, _ | .floatNan, _ | .floatInf _, _ => none
  | _, .float .. | _, .f64 _ | _, .floatNan | _, .floatInf _ => none
  | .closure .., _ | _, .closure .. | .poison _, _ | _, .poison _ => none
  | _, _ => some (compare (valRank a) (valRank b))

/-- Lexicographic order over a value sequence; prefix-equal → the shorter sequence is less. -/
partial def cmpValSeq (xs ys : Array Value) (i : Nat) : Option Ordering :=
  if i < xs.size && i < ys.size then
    match cmpValue (xs[i]!) (ys[i]!) with
    | some .eq => cmpValSeq xs ys (i + 1)
    | r => r
  else some (compare xs.size ys.size)

/-- Order sorted record fields lexicographically (key bytes, then value). -/
partial def cmpValFields (fs gs : Array (ByteArray × Value)) (i : Nat) : Option Ordering :=
  if i < fs.size && i < gs.size then
    match cmpBytes (fs[i]!.1) (gs[i]!.1) with
    | .eq => (match cmpValue (fs[i]!.2) (gs[i]!.2) with | some .eq => cmpValFields fs gs (i + 1) | r => r)
    | o => some o
  else some (compare fs.size gs.size)

/-- Order sorted map entries lexicographically (key, then value). -/
partial def cmpValMapEntries (xs ys : Array (Value × Value)) (i : Nat) : Option Ordering :=
  if i < xs.size && i < ys.size then
    match cmpValue (xs[i]!.1) (ys[i]!.1) with
    | some .eq => (match cmpValue (xs[i]!.2) (ys[i]!.2) with | some .eq => cmpValMapEntries xs ys (i + 1) | r => r)
    | r => r
  else some (compare xs.size ys.size)
end

/-- CANONICAL structural equality for set/map KEYS & elements (`cmpValue a b == .eq`): unlike raw `BEq`, it
unifies float FORMS (`.f64 1.5` = `.float 1.5`), folds all NaN equal, and keeps `-0.0` ≠ `0.0` — so a
computed float key is found by its literal, and NaN keys dedupe (spec CHAMP-canonical key equality). A
float/closure/poison-free compound compares exactly as `==`. -/
def valEq (a b : Value) : Bool := cmpValue a b == some Ordering.eq

/-- Build a NORMALIZED exact `Rational` value `num/den`: `none` if `den == 0` (the caller traps
div-by-zero). Otherwise the sign is moved to the numerator (den > 0) and both are divided by their gcd,
so structural `BEq` on the result is value equality (spec §"An Exact Rational Has A Canonical Normalized
Form"). `0/d` normalizes to `0/1`. -/
def mkRational (num den : Int) : Option Value :=
  if den == 0 then none
  else
    let (n, d) := if den < 0 then (-num, -den) else (num, den)   -- d > 0
    let g : Int := Int.ofNat (n.gcd d)                            -- gcd(|n|, d) ≥ 1 (=d when n=0)
    let g := if g == 0 then 1 else g
    some (.rational (n / g) (d / g))

/-- Exact rational arithmetic `(a/b) op (c/d)` with `b, d > 0` (normalized) → a normalized Rational. `/`
by a ZERO rational (`c == 0`) traps div-by-zero; `+ - *` never divide by zero (b·d > 0). -/
def rationalArith (op : String) (a b c d : Int) : Outcome :=
  match op with
  | "+" => (match mkRational (a * d + c * b) (b * d) with | some v => .value v | none => .trap "div-by-zero")
  | "-" => (match mkRational (a * d - c * b) (b * d) with | some v => .value v | none => .trap "div-by-zero")
  | "*" => (match mkRational (a * c) (b * d) with | some v => .value v | none => .trap "div-by-zero")
  | "/" => if c == 0 then .trap "div-by-zero"
           else (match mkRational (a * d) (b * c) with | some v => .value v | none => .trap "div-by-zero")
  | _ => .unsupported s!"eval: rational operator {op} not modeled"

/-- ENV-AWARE operand-width inference: a `(: e T)` ascription gives its integer type; an arithmetic op is
BigInt if EITHER operand is (BigInt is contagious — unbounded, no overflow); a qualified `((. BigInt …) …)`
call is BigInt; and a bare-NAME operand consults its binding's stored `IntTy` (a param ascription or a
BigInt-typed let/do binding). So an operation takes its width from its operands (e.g. `(+ (: v UInt64) …)`
is UInt64 arithmetic, not the ambient default). The bare-name case is what lets BigInt-ness propagate
through let-var CHAINS: `(let ((q (/ n d)) (r (% n d))) (+ (* q d) r))` — `q`/`r`'s values are arithmetic
over the BigInt let-vars `n`/`d`, so their bindings resolve BigInt, and the outer `(+ (* q d) r)` infers
BigInt too rather than defaulting to Int64 and false-overflowing a multi-limb value (an env-less inference
returned `none` for any bare-name operand → the multi-limb division identity 06-numeric 0215/0255 mis-inferred
Int64 and trapped overflow). A minimal bottom-up inference for the scalar core. -/
partial def operandTyEnv? (m : Module) (env : Env) (i : Nat) : Option IntTy :=
  match m.nodes[i]? with
  | some (Node.atom lid) =>
    match m.leaves[lid]? with
    | some (Leaf.name nm) => (env.lookup? nm).bind (·.2)
    -- an `N`-suffixed BigInt literal operand is BigInt-typed → arith over it is unbounded (no overflow).
    | some (Leaf.suffixed 0 _) => some { signed := true, width := .big }
    | _ => none
  | some (Node.list cs) =>
    match m.headName? (Node.list cs) with
    | some h =>
      if h == ":".toUTF8 && cs.size ≥ 3 then parseIntTy? m cs[2]!
      else if arithOps.contains ((String.fromUTF8? h).getD "") &&
              (((cs[1]?).bind (operandTyEnv? m env)).any (·.width == .big) ||
               ((cs[2]?).bind (operandTyEnv? m env)).any (·.width == .big)) then
        some { signed := true, width := .big }
      else none
    | none =>
      match (cs[0]?).bind (fun hid => m.nodes[hid]?) with
      | some (Node.list hc) =>
        if m.headName? (Node.list hc) == some ".".toUTF8 then
          match (hc[1]?).bind (nameOf? m), (hc[2]?).bind (nameOf? m) with
          -- a qualified `((. BigInt …) …)` call is BigInt (unbounded); a fixed-width conversion
          -- `((. <IntTy> wrap|of) …)` yields a value OF that IntTy, so a checked op over it guards at that
          -- (narrow) width — `(+ (UInt8.wrap a) (UInt8.wrap b))` guards at 8 (06-numeric wrapped-byte add).
          | some q, some mem =>
            if q == "BigInt".toUTF8 then some { signed := true, width := .big }
            else if mem == "wrap".toUTF8 || mem == "of".toUTF8 then parseIntTyName? q
            else none
          | _, _ => none
        else none
      | _ => none
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
  -- a `?` short-circuit reaching a LAZY store position (e.g. a `(try …)` as a tuple/record element, or a
  -- non-try-containing binding) is an unmodeled control-flow-in-a-lazy-slot shape → a poison (sound skip
  -- if ever observed). A try in a STRICT position (list element #5194, let binding, fn body) short-circuits
  -- eagerly and never reaches here.
  | .errReturn _ => .poison (.unsupported "try short-circuit reached a lazy element position (not modeled)")

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
  else
    -- USER sum ctors SHADOW prelude ctors of the same name (corpus 05-compound bare-collision doc): a
    -- user variant reusing a prelude DATA-CONSTRUCTOR name (Sign Neg/Pos/Zero, Ordering Less/Equal/
    -- Greater) binds to the LOCAL variant, not the prelude nullary ctor — the built-in sums inject
    -- their ctor names into the prelude AFTER the variant-ctor index snapshot. So resolve against the
    -- user `(type …)` decls FIRST (a name declared by SOME user type NEVER falls back to the prelude,
    -- even if its own arity is unmodeled → none); only a name no user type declares uses the prelude.
    -- (Was prelude-first, which mis-read a user `(Neg T)` as the prelude's nullary Neg → dropped its
    -- payload to unit — 03-0094 mutually-recursive-sums.)
    match (userSumTypes m).findSome? (fun (_, ctors) =>
            (ctors.find? (fun c => c.1 == name)).map (fun c => (ctors.length, c.2))) with
    | some (nvariants, ar) =>
      -- a SINGLE-variant type with ≥1 field(s) ERASES (no tag): arity 1 → its field (newtypeCtor?), arity
      -- ≥2 → the tuple of its fields (structNewtypeCtor?) — 05-compound "a multi-payload struct newtype
      -- escapes as its payload tuple". So it is NOT a tagged variant → none here. A MULTI-variant sum's
      -- ctor IS tagged (`variant C payload`, payload = unit/field/tuple by arity) → some ar. A SOLE NULLARY
      -- ctor (nvariants==1, ar==0) → some 0 (evalVariantCtor erases it to unit via soleNullaryCtor?).
      if nvariants == 1 && ar ≥ 1 then none else some ar
    | none => (preludeSumCtors.find? (fun p => name == p.1.toUTF8)).map (·.2)

/-- Is `name` a NEWTYPE constructor — the SOLE variant of its user type, carrying EXACTLY ONE field?
Such a sum SCALAR-ERASES: its value IS the payload, construction is identity, a pattern binds the
payload directly (spec type-system.md §"A Single-Variant Single-Field Sum Is A Nominal Type Over Its
Payload", #4516). A multi-variant / nullary / multi-field ctor is NOT a newtype (stays tagged). A
def-shadowed name is not a ctor. -/
def newtypeCtor? (m : Module) (name : ByteArray) : Bool :=
  !((defNames m).contains name) &&
  (userSumTypes m).any (fun (_, ctors) => match ctors with | [(cn, 1)] => cn == name | _ => false)

/-- A SOLE NULLARY constructor: a type with exactly ONE ctor which is nullary (`(type T (A))`). Like a
newtype (single-field → erase to the field), a single-nullary-ctor type carries no information, so its
value ERASES to `unit` — `(T.A)` = `(: unit T)` — NOT a tagged `variant` (a nullary ctor of a MULTI-ctor
type, e.g. Option's `None`, stays tagged to distinguish it from its siblings). (corpus 11-modules 0023.) -/
def soleNullaryCtor? (m : Module) (name : ByteArray) : Bool :=
  !((defNames m).contains name) &&
  (userSumTypes m).any (fun (_, ctors) => match ctors with | [(cn, 0)] => cn == name | _ => false)

/-- A STRUCT NEWTYPE ctor: the SOLE ctor of its type, with ≥2 fields (`(type Pt (Mk Int64 Int64))`). Like a
newtype (single-field → its field) it SCALAR-ERASES — its value is the bare TUPLE of its fields (tag erased
into the type header), NOT a tagged `variant` (05-compound "a multi-payload struct newtype escapes as its
payload tuple"). A multi-field ctor of a MULTI-variant sum keeps its tag (that path stays a `variant`). -/
def structNewtypeCtor? (m : Module) (name : ByteArray) : Bool :=
  !((defNames m).contains name) &&
  (userSumTypes m).any (fun (_, ctors) => match ctors with | [(cn, ar)] => cn == name && ar ≥ 2 | _ => false)

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
  -- `cmpValue` is a total STRUCTURAL order (also over COMPOUND elements — sets of tuples/lists/records/…);
  -- `none` only on a float/closure/poison element (stays unorderable → skip).
  if elems.all (fun e => (cmpValue e e).isSome) then
    let sorted := elems.qsort (fun a b => cmpValue a b == some Ordering.lt)
    some (sorted.foldl (fun acc e => if acc.size > 0 && valEq (acc[acc.size - 1]!) e then acc else acc.push e) #[])
  else none

/-- Canonicalize a Map's entries: require every KEY be orderable, SORT by key, dedupe by key (a later
entry wins — the canonical Map form is sorted-by-key with unique keys). `none` on an unorderable key. -/
def canonMap (entries : Array (Value × Value)) : Option (Array (Value × Value)) :=
  if entries.all (fun e => (cmpValue e.1 e.1).isSome) then
    -- LAST-insert-wins per key, in INSERTION order, THEN sort by key. Deduping BEFORE the sort makes the
    -- survivor independent of qsort stability — a duplicate key `(map (= n 1) (= n 2))` must keep the LAST
    -- value (2), not an arbitrary one (06-numeric cdzw19: last-insert-wins survives the stored-order replay).
    let deduped := entries.foldl (fun acc e =>
      match acc.findIdx? (fun a => valEq a.1 e.1) with
      | some i => acc.set! i e
      | none => acc.push e) #[]
    some (deduped.qsort (fun a b => cmpValue a.1 b.1 == some Ordering.lt))
  else none

/-- Recursively put a Value into CANONICAL form: SETS sorted+deduped (`canonSet`), MAPS sorted-by-key+deduped
(`canonMap`), RECORD fields sorted by key bytes — and every compound CHILD canonicalized FIRST (so the parent
sort sees canonical children). This is the helper the wasm heap-result DECODER (v-wasm-oracle, W5.5) calls so a
value decoded from the final `HeapState` in arbitrary (insertion/hash) order matches Core's canonical form
under the order-SENSITIVE `Value.valueEqSpec` — without it a decoded set/map/record would false-diverge on
ordering. An UNORDERABLE set/map (a float mixed with non-floats, or a closure/poison key → `canonSet`/`canonMap`
return `none`) is left AS-IS: such a value never reaches a successful compare on either side (Core's own
construction declines it too), so leaving it unsorted is sound. -/
partial def canonicalizeValue : Value → Value
  | .set es =>
    let es' := es.map canonicalizeValue
    (match canonSet es' with | some c => .set c | none => .set es')
  | .map ps =>
    let ps' := ps.map (fun e => (canonicalizeValue e.1, canonicalizeValue e.2))
    (match canonMap ps' with | some c => .map c | none => .map ps')
  | .record fs =>
    let fs' := fs.map (fun e => (e.1, canonicalizeValue e.2))
    .record (fs'.qsort (fun a b => cmpBytes a.1 b.1 == .lt))
  | .tuple es    => .tuple (es.map canonicalizeValue)
  | .list es     => .list (es.map canonicalizeValue)
  | .some v      => .some (canonicalizeValue v)
  | .ok v        => .ok (canonicalizeValue v)
  | .err v       => .err (canonicalizeValue v)
  | .variant t v => .variant t (canonicalizeValue v)
  | v            => v

-- canonicalizeValue: an out-of-order set with a duplicate → sorted + deduped.
#guard (canonicalizeValue (.set #[.int 3, .int 1, .int 2, .int 1]) == .set #[.int 1, .int 2, .int 3])
-- record fields reordered to canonical key-sorted order.
#guard (canonicalizeValue (.record #[("b".toUTF8, .int 1), ("a".toUTF8, .int 2)])
        == .record #[("a".toUTF8, .int 2), ("b".toUTF8, .int 1)])
-- NESTED: a set inside a tuple is canonicalized recursively (child sorted, then the tuple preserved).
#guard (canonicalizeValue (.tuple #[.set #[.int 2, .int 1], .int 9])
        == .tuple #[.set #[.int 1, .int 2], .int 9])
-- map entries sorted by key, last-value-wins dedupe (canonMap semantics).
#guard (canonicalizeValue (.map #[(.int 2, .str "b".toUTF8), (.int 1, .str "a".toUTF8)])
        == .map #[(.int 1, .str "a".toUTF8), (.int 2, .str "b".toUTF8)])

/-- `Map.insert m k v`: replace any existing entry for `k`, then add `k ↦ v` (canonicalized by `canonMap`).
Key equality MUST be `valEq` (canonical, bit-exact via `cmpValue`), NOT the derived `Value` `==`: for floats
the derived BEq is IEEE equality where positive-zero and negative-zero compare EQUAL, which would COLLAPSE
them into one key on insert — but the spec keeps signed zeros DISTINCT (sign-significant zero) and
`canonMap`/`Map.lookup` use bit-exact `valEq`. The `==` here was inconsistent with lookup: inserting the two
signed zeros dropped the positive-zero entry, then a positive-zero lookup (valEq) missed it, a wrong result
(differential 19-sets-0259 "nz4 Map discriminates signed zeros": Core -2 vs wasm 1020). `valEq` makes insert
discriminate consistently with lookup + spec. -/
def mapInsertRaw (entries : Array (Value × Value)) (k v : Value) : Array (Value × Value) :=
  (entries.filter (fun e => !(valEq e.1 k))).push (k, v)

-- Map insert discriminates the two signed zeros as DISTINCT keys (bit-exact valEq, not IEEE `==`) — nz4 fix.
#guard ((mapInsertRaw (mapInsertRaw #[] (.f64 0.0) (.int 10)) (.f64 (-0.0)) (.int 20)).size == 2)
-- but a genuine same-key re-insert (both +0.0) REPLACES (last-writer), size 1.
#guard ((mapInsertRaw (mapInsertRaw #[] (.f64 0.0) (.int 10)) (.f64 0.0) (.int 20)).size == 1)

/-- Structurally REFLECT the AST subtree at node `i` into an `Ast` sum VALUE, WITHOUT evaluating it
(`metaprogramming.md` #Quote Produces An AST Value: `(quote <expr>)` returns the `Ast` value for
<expr>'s structure — UNCONDITIONALLY, whatever <expr> contains, so a nested `quasiquote`/`unquote` is
inert literal structure, not an active splice). Each leaf becomes its matching `Ast` variant (the
prelude `Ast` sum's variants: `Int`/`Float`/`Bool`/`Str`/`Name`/`Bytes`/`Char`; a NAME reflects to
`Ast.Name` carrying the identifier text, distinct from a string literal's `Ast.Str`), and a list node
becomes `Ast.List` of the reflected children. These are the SAME `variant` values the qualified
constructor `(. Ast Ctor)` builds, so a quoted value compares equal to its written-out `Ast.*` form.
A `Symbol`/suffixed/malformed leaf is not modeled as a value → `unsupported` (a sound skip: the whole
quoted structure cannot be faithfully represented). -/
partial def quoteReflect (m : Module) (fuel : Nat) (i : Nat) : Outcome :=
  match fuel with
  | 0 => .diverges
  | Nat.succ fuel' =>
    match m.nodes[i]? with
    | some (Node.atom lid) =>
      match m.leaves[lid]? with
      | some (.intLit neg _ mag) =>
        let n := Int.ofNat (Value.beBytesToNat mag)
        .value (Value.variant "Int".toUTF8 (Value.int (if neg then -n else n)))
      | some (.float neg e s) => .value (Value.variant "Float".toUTF8 (Value.float neg e s))
      | some .floatNan => .value (Value.variant "Float".toUTF8 Value.floatNan)
      | some (.floatInf neg) => .value (Value.variant "Float".toUTF8 (Value.floatInf neg))
      | some (.boolLit b) => .value (Value.variant "Bool".toUTF8 (Value.bool b))
      | some (.str b) => .value (Value.variant "Str".toUTF8 (Value.str b))
      | some (.name b) => .value (Value.variant "Name".toUTF8 (Value.str b))
      | some (.bytesLit b) => .value (Value.variant "Bytes".toUTF8 (Value.bytes b))
      | some (.char b) => .value (Value.variant "Char".toUTF8 (Value.char b))
      | some _ => .unsupported "quote: unmodeled leaf (symbol/suffixed/malformed) in quoted structure"
      | none => .unsupported "quote: leaf index out of range"
    | some (Node.list children) =>
      -- reflect each child into an `Ast.List`; any unmodeled child short-circuits to its outcome
      let reflected : Except Outcome (Array Value) :=
        children.foldl (fun acc j =>
          match acc with
          | .error o => .error o
          | .ok vs =>
            match quoteReflect m fuel' j with
            | .value v => .ok (vs.push v)
            | other => .error other) (.ok #[])
      match reflected with
      | .ok vs => .value (Value.variant "List".toUTF8 (Value.list vs))
      | .error o => o
    | none => .unsupported "quote: node index out of range"

/-- Lift an unquote-SPLICED value into its `Ast` representation for embedding in a quasiquote result.
A scalar becomes its matching `Ast` variant (corpus-verified: `,x` with x=7 → `(Ast.Int 7)`); a value
that is ALREADY an `Ast` variant (one of the 9 ctor tags) embeds as-is; a compound/closure/poison cannot
be faithfully lifted to a syntax node → `unsupported` (a sound skip). -/
def valueToAst : Value → Outcome
  | .int n => .value (Value.variant "Int".toUTF8 (Value.int n))
  | .float a b c => .value (Value.variant "Float".toUTF8 (Value.float a b c))
  | .floatNan => .value (Value.variant "Float".toUTF8 Value.floatNan)
  | .floatInf n => .value (Value.variant "Float".toUTF8 (Value.floatInf n))
  | .bool b => .value (Value.variant "Bool".toUTF8 (Value.bool b))
  | .str b => .value (Value.variant "Str".toUTF8 (Value.str b))
  | .char b => .value (Value.variant "Char".toUTF8 (Value.char b))
  | .bytes b => .value (Value.variant "Bytes".toUTF8 (Value.bytes b))
  | .variant tag p =>
    if ["Int", "Float", "Bool", "Str", "Name", "List", "Bytes", "Char", "Symbol"].contains ((String.fromUTF8? tag).getD "")
    then .value (Value.variant tag p) else .unsupported "quasiquote: cannot splice a non-Ast variant value"
  | _ => .unsupported "quasiquote: cannot splice a compound/unmodeled value into an AST"

/-- REIFY an `Ast` VALUE (a reflected syntax tree — the `variant "Int"/"Name"/"List"/…` shape `quote`/
`quasiquote` produce) back into concrete AST nodes, APPENDED to module `m`, returning the extended module
and the reified subtree's ROOT node id. This is the inverse of `quoteReflect`: `Ast.Name` → a `name` atom
(NOT `str` — an identifier, so it resolves as a reference), `Ast.Str` → a `str` atom, `Ast.Int/Bool/…` →
their scalar leaf, `Ast.List` → a `list` node over the reified children. Appending (rather than building a
standalone module) PRESERVES `m`'s scope — the reified expression's ctor/def resolution still scans `m`'s
top-level `(do …)`, so `(eval (quote (S (S (Z)))))` resolves `S`/`Z` against the program's `type` decls.
A non-`Ast` / unmodeled value (a `Symbol`, a compound, a non-Ast variant) → `error` (a sound eval skip). -/
partial def reifyInto (m : Module) (v : Value) : Except String (Module × Nat) :=
  match v with
  | .variant tag payload =>
    let tagS := (String.fromUTF8? tag).getD ""
    if tagS == "List" then
      match payload with
      | .list elems =>
        (Array.foldlM (fun (st : Module × Array Nat) e => do
            let (mod, cid) ← reifyInto st.1 e
            pure (mod, st.2.push cid)) ((m, (#[] : Array Nat))) elems).bind
          (fun (mod, kids) =>
            let nid := mod.nodes.size
            .ok ({ mod with nodes := mod.nodes.push (Node.list kids) }, nid))
      | _ => .error "eval: Ast.List payload is not a list"
    else if tagS == "Name" then
      match payload with
      | .str b =>
        let lid := m.leaves.size
        let m := { m with leaves := m.leaves.push (Leaf.name b) }
        let nid := m.nodes.size
        .ok ({ m with nodes := m.nodes.push (Node.atom lid) }, nid)
      | _ => .error "eval: Ast.Name payload is not a string"
    else
      -- a scalar variant (Int/Float/Bool/Str/Char/Bytes): rebuild its leaf from the payload value.
      match payload.toLeaf? with
      | some leaf =>
        let lid := m.leaves.size
        let m := { m with leaves := m.leaves.push leaf }
        let nid := m.nodes.size
        .ok ({ m with nodes := m.nodes.push (Node.atom lid) }, nid)
      | none => .error s!"eval: cannot reify Ast.{tagS} payload"
  | _ => .error "eval: value is not an Ast node (cannot reify)"

/-- Does an effect op's type return `Unit`? The type is either an arrow `(-> Arg… Ret)` (Ret = last
child) or a bare type; Unit-returning iff that return position is the name `Unit`. -/
def retIsUnit? (m : Module) (tyId : Nat) : Bool :=
  match m.nodes[tyId]? with
  | some (Node.list tc) =>
    if m.headName? (Node.list tc) == some "->".toUTF8 && tc.size ≥ 2
    then (tc[tc.size - 1]?).bind (nameOf? m) == some "Unit".toUTF8
    else false
  | some (Node.atom _) => (nameOf? m tyId) == some "Unit".toUTF8
  | _ => false

/-- Resolve a performed effect op `((. eff op) …)` against the program's `(effect eff (op op <type>)…)`
declarations: `some true` if the op RETURNS Unit (its perform yields `unit` — response-independent, so
modelable purely NOW), `some false` if it returns a VALUE (the perform's result IS the host response —
needs host-response threading, H2, so unmodeled here → skip), `none` if `(eff, op)` is not a declared
effect op. This is the H1a foundation of host-function modeling (the L2 WIT/host surface). -/
def effectOpRetUnit? (m : Module) (eff op : ByteArray) : Option Bool :=
  match m.nodes[m.root]? with
  | some (Node.list stmts) =>
    stmts.findSome? (fun sid =>
      match m.nodes[sid]? with
      | some (Node.list ec) =>
        if m.headName? (Node.list ec) == some "effect".toUTF8 && (ec[1]?).bind (nameOf? m) == some eff then
          (ec.extract 2 ec.size).findSome? (fun oid =>
            match m.nodes[oid]? with
            | some (Node.list oc) =>
              if m.headName? (Node.list oc) == some "op".toUTF8 && (oc[1]?).bind (nameOf? m) == some op then
                some (match oc[2]? with | some tyId => retIsUnit? m tyId | none => true)
              else none
            | _ => none)
        else none
      | _ => none)
  | _ => none

mutual
/-- SHORT-CIRCUIT structural equality (spec core-semantics.md §Equality Is Structural: equality is
component-wise + §A Trap Occurs Only Where Its Computation Is Observed: an unobserved subcomputation's
trap MAY be elided). Compare component-wise, forcing each component only until the result is DECIDED —
the FIRST differing component stops the walk, so a LATER compound element (a `poison` deferring a trap)
is never observed and its trap never surfaces. This matches rcdzc's short-circuit; the previous
`observeDeep`-both-then-`==` OVER-FORCED — it surfaced a trapping later element even when an earlier
component already decided inequality (a systematic value-vs-trap divergence the L2 differential found).
A `poison` reached while the result is still undecided (its component IS observed) surfaces its outcome.
Scalars / sets / maps (canonical, no deferred element) compare via `Value.valueEqSpec` (f64-aware). -/
partial def eqSC (a b : Value) : Outcome :=
  match a, b with
  | .poison d, _ => deferredToOutcome d
  | _, .poison d => deferredToOutcome d
  | .some x, .some y => eqSC x y
  | .ok x, .ok y => eqSC x y
  | .err x, .err y => eqSC x y
  | .variant t1 p1, .variant t2 p2 => if t1 == t2 then eqSC p1 p2 else .value (.bool false)
  | .tuple xs, .tuple ys => if xs.size == ys.size then eqSeqSC xs ys 0 else .value (.bool false)
  | .list xs, .list ys => if xs.size == ys.size then eqSeqSC xs ys 0 else .value (.bool false)
  | .record f1, .record f2 =>
    -- fields are canonical (sorted by key); equal iff same key set AND field-wise value-equal (short-circuit)
    if f1.size == f2.size && (f1.zip f2).all (fun p => p.1.1 == p.2.1)
    then eqSeqSC (f1.map (·.2)) (f2.map (·.2)) 0 else .value (.bool false)
  | _, _ => .value (.bool (Value.valueEqSpec a b))

/-- Component-wise short-circuit for `eqSC`: the first FALSE stops the walk (later elements unobserved);
a trap/unsupported/diverges from an observed component propagates. -/
partial def eqSeqSC (xs ys : Array Value) (i : Nat) : Outcome :=
  if i < xs.size then
    match eqSC (xs[i]!) (ys[i]!) with
    | .value (.bool true) => eqSeqSC xs ys (i + 1)
    | other => other
  else .value (.bool true)
end

/-- A resolved call-argument SOURCE after splat expansion: either an unevaluated arg NODE (bound LAZILY
under its parameter, per spec — an unused parameter's arg is never forced) or a VALUE already produced by
expanding a splat `(.. tuple)` operand (the operand is evaluated STRICTLY to spread its elements into
per-slot arguments, matching the compiler's `(a3 (. t 0) (. t 1) …)` projection expansion). -/
inductive ArgSrc where
  | node : Nat → ArgSrc
  | val  : Value → ArgSrc

/-- Is call-argument node `aid` a SPLAT `(.. e)` (head `..`, exactly one operand)? Pure (no eval) — used
at the call-dispatch to route a splat-carrying call to `evalCallSplat`. Returns the operand node id. -/
def splatOperand? (m : Module) (aid : Nat) : Option Nat :=
  match m.nodes[aid]? with
  | some (Node.list c) =>
    if m.headName? (Node.list c) == some "..".toUTF8 && c.size == 2 then c[1]? else none
  | _ => none

/-- Does any call argument (children after the head) splat? -/
def hasSplatArg (m : Module) (children : Array Nat) : Bool :=
  (children.extract 1 children.size).any (fun aid => (splatOperand? m aid).isSome)

/-- Detect a TRAILING rest binder among a tuple/record sub-pattern array `sps`. Two spellings occur: a
GROUPED `(.. binder)` node (surface `#tuple(a (.. rest))` / `#record((= x a) (.. rest))` — head `..`, its
one operand is the binder) or a BARE `..` marker element followed by its binder (the list-pattern spelling
`(list x .. r)`). Returns `(leadingCount, binderNodeId)` — how many leading positional/field patterns
precede the rest, and the node the residual binds to. `none` = no rest marker present. -/
def restBinderOf? (m : Module) (sps : Array Nat) : Option (Nat × Nat) :=
  match sps.findIdx? (fun sp => (m.nodes[sp]?).bind (fun n => m.headName? n) == some "..".toUTF8) with
  | some k =>  -- grouped `(.. binder)` at index k
    match m.nodes[sps[k]!]? with
    | some (Node.list gc) => (gc[1]?).map (fun binder => (k, binder))
    | _ => none
  | none =>  -- bare `..` NAME element, binder is the next element
    match sps.findIdx? (fun sp => nameOf? m sp == some "..".toUTF8) with
    | some k => (sps[k+1]?).map (fun binder => (k, binder))
    | none => none

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
        -- a bare name: force its (lazy) binding; else a bare NULLARY CONSTRUCTOR used as a value (`None`,
        -- or a user/prelude nullary ctor) — the value form of `(None)`/`(C)` written without parens; else
        -- an (unmodeled) free/prelude name.
        match env.lookup? b with
        | some (thunk, _) => thunk.get  -- propagates the binding's value / trap / unsupported / diverges
        | none =>
          if b == "None".toUTF8 then .value Value.none
          else match variantCtorArity? m b with
               | some 0 => .value (if soleNullaryCtor? m b then .unit else .variant b .unit)
               | _ => .unsupported "eval: free name (variable not bound; prelude/global not yet modeled)"
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
           -- a single-variant ≥2-field STRUCT newtype erases to the bare TUPLE of its fields (no tag).
           else if structNewtypeCtor? m cname then some (evalSeqCtor m env fuel children Value.tuple false)
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
               -- prelude Ast sum ctors, QUALIFIED-ONLY `(. Ast Int|Name|Str|Bool|List|Bytes|Char|Symbol)` (their
               -- names collide with type/module names, so they are never bare). Each carries one payload →
               -- a generic `variant`. (Metaprog foundation: quote reflects an AST into these Ast values.)
               else if q == "Ast".toUTF8 &&
                       ["Int", "Float", "Name", "Str", "Bool", "List", "Bytes", "Char", "Symbol"].contains ((String.fromUTF8? c).getD "") then
                 some (evalUnaryCtor m env fuel children (Value.variant c))
               -- PERFORM a declared effect op `((. eff op) args)` (host-function modeling, H1a): a
               -- Unit-returning op yields `unit` (response-independent) after its args are observed; a
               -- value-returning op's result IS the host response (H2) → skip until responses are threaded.
               else match effectOpRetUnit? m q c with
                 | some true => some (performUnitOp m env fuel children)
                 | some false => some (.unsupported "eval: value-returning effect op (host response not yet modeled)")
                 | none => none
             | none => none)
        -- a BARE (unapplied) member-access `(. T C)` naming a NULLARY constructor — the node's own head is
        -- `.` (distinct from the APPLIED `((. T C) arg)` caught above via ctorAppName/qualHead). `(. Option
        -- None)` (built-in or a user `(type Option … None)` redeclaration) unifies onto Value.none; any other
        -- declared nullary ctor → its tagged nullary variant. Guard: if `T` is a locally BOUND value this is
        -- a record projection `(. rec field)`, not a ctor ref → leave it for evalProject.
        <|> (match m.headName? (Node.list children) with
             | some dh =>
               if dh == ".".toUTF8 && children.size == 3 then
                 match (children[1]?).bind (nameOf? m), (children[2]?).bind (nameOf? m) with
                 | some q, some mem =>
                   if (env.lookup? q).isSome then none
                   else if mem == "None".toUTF8 then some (Outcome.value Value.none)
                   else match variantCtorArity? m mem with
                        | some 0 => some (Outcome.value (if soleNullaryCtor? m mem then .unit else .variant mem .unit))
                        | _ => none
                 | _, _ => none
               else none
             | none => none)
        <|> ((qualHead? m children).bind (fun (q, f) => evalModuleFn m env fuel q f children))
        <|> ((m.headName? (Node.list children)).bind (fun h =>
               if (env.lookup? h).isSome then none                     -- a local binding shadows: not a top-level call
               else (defTable m).find? (fun d => d.1 == h) |>.bind (fun d =>
                 let params := d.2.1; let nargs := children.size - 1
                 -- A SPLAT argument `(.. tuple)` supplies several positional args from ONE child, so the raw
                 -- child count `nargs` under-counts — expand+bind via `evalCallSplat` (checked route first,
                 -- else a 3-param call with a single tuple-splat arg would mis-fire the partial-app branch).
                 if hasSplatArg m children then some (evalCallSplat m env fuel params d.2.2 children)
                 else if params.size == nargs then some (evalCall m env fuel params d.2.2 children)
                 -- PARTIAL application `(f a)` (f has more params than args given): a closure over the
                 -- REMAINING params, CAPTURING the given args (as values, evaluated now) under their param
                 -- names. Applied later (`((f a) b)`), applyClosure binds the rest + this cap.
                 else if 0 < nargs && nargs < params.size then
                   let cap := ((params.extract 0 nargs).zip (children.extract 1 children.size)).toList.filterMap
                     (fun (sp, aid) => (paramSpec? m sp).map (fun (nm, _) =>
                       (nm, outcomeToValue (evalNode m env defaultIntTy fuel aid))))
                   some (.value (.closure (params.extract nargs params.size) d.2.2 cap))
                 else none)))
      match ctorConstruct with
      | some o => o
      | none =>
      match m.headName? (Node.list children) with
      | some h =>
        if h == "let".toUTF8 then evalLet m env ty fuel children
        else if h == "do".toUTF8 then evalDo m env ty fuel children
        else if h == "host".toUTF8 then
          -- `(host (caps) body)`: the host context provides capabilities to `body`; evaluate the body
          -- (the effect ops themselves are resolved from the `(effect …)` decls, not the caps list).
          (match children[2]? with | some b => evalNode m env ty fuel b | none => .unsupported "eval: malformed host")
        else if h == "quote".toUTF8 then
          -- `(quote <expr>)`: reflect the single quoted subtree structurally, WITHOUT evaluating it.
          match children[1]? with
          | some c => quoteReflect m fuel c
          | none => .unsupported "quote: missing quoted argument"
        else if h == "quasiquote".toUTF8 then
          -- `(quasiquote <body>)`: reflect the body at nesting level 1 — active unquotes splice, deeper
          -- ones stay structure. The top quasiquote is consumed (replaced by its reflected body).
          match children[1]? with
          | some c => evalQuasi m env 1 c
          | none => .unsupported "quasiquote: missing body"
        else if h == "eval".toUTF8 then
          -- `(eval <ast-expr>)`: evaluate the argument to an Ast VALUE, REIFY it back into concrete nodes
          -- (appended to `m`, so the program's scope/ctors/defs are preserved), then evaluate that node.
          match children[1]? with
          | some c =>
            match evalNode m env ty fuel c with
            | .value astv =>
              match reifyInto m astv with
              | .ok (m', rootId) => evalNode m' env defaultIntTy fuel rootId
              | .error e => .unsupported e
            | other => other
          | none => .unsupported "eval: missing argument"
        else if h == "if".toUTF8 then evalIf m env ty fuel children
        else if h == ":".toUTF8 then evalAscribe m env ty fuel children
        else if h == "fn".toUTF8 then evalFn m env fuel children
        else if h == "try".toUTF8 then evalTry m env fuel children
        else if (env.lookup? h).isSome then
          -- the head is a BOUND local — an application of that binding. If it forces to a CLOSURE,
          -- apply it; otherwise (a non-function value applied) it is not modeled → skip.
          match ((env.lookup? h).map (fun e => e.1.get)).getD (.unsupported "") with
          | .value (.closure params body cap) => applyClosure m env fuel params body cap children
          | .value _ => .unsupported "eval: head is a bound non-function value"
          | other => other
        else if h == "Some".toUTF8 then evalUnaryCtor m env fuel children Value.some
        else if h == "Ok".toUTF8 then evalUnaryCtor m env fuel children Value.ok
        else if h == "Err".toUTF8 then evalUnaryCtor m env fuel children Value.err
        else if h == "None".toUTF8 then Outcome.value Value.none
        else if h == "tuple".toUTF8 then evalSeqCtor m env fuel children Value.tuple false
        else if h == "list".toUTF8 then evalSeqCtor m env fuel children Value.list true
        else if h == "record".toUTF8 then evalRecord m env fuel children
        else if h == "map".toUTF8 then evalMapLiteral m env fuel children
        else if h == "set".toUTF8 then evalSetLiteral m env fuel children
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
      | none =>
        -- a NON-NAME head: the head is itself an EXPRESSION — an immediately-applied lambda
        -- `((fn (x) …) a)` or a computed function `((f x) y)`. Evaluate it; if it forces to a closure,
        -- apply it to the remaining children. (A member-access head like `(. Ast print)` evaluates via
        -- `evalProject` and yields a non-closure → unsupported, a sound skip.)
        match children[0]? with
        | some hid =>
          match evalNode m env defaultIntTy fuel hid with
          | .value (.closure params body cap) => applyClosure m env fuel params body cap children
          -- a NULLARY application `(e)` (single child, no args) of a non-closure value = that value: a
          -- zero-arg module value/ctor called in call position, e.g. `(Map.empty)` = `((. Map empty))`.
          | .value v => if children.size == 1 then .value v
                        else .unsupported "eval: applied a non-function computed head"
          | other => other
        | none => .unsupported "eval: empty list"
    | none => .unsupported "eval: node index out of range"

/-- Reflect a quasiquote body at nesting `level` (≥1) into an `Ast` value (`metaprogramming.md`
#Quasiquote Constructs AST With Selective Evaluation). Standard level arithmetic: an `(unquote E)` at
level 1 is ACTIVE — evaluate `E` and SPLICE its value (lifted via `valueToAst`); a DEEPER unquote
DECREMENTS the level and stays literal `(unquote …)` structure. A nested `(quasiquote Y)` INCREMENTS the
level and stays literal `(quasiquote …)` structure. Any other list reflects its children at the SAME
level into `Ast.List`; a leaf reflects to its `Ast` variant (as `quoteReflect`). -/
partial def evalQuasi (m : Module) (env : Env) (level : Nat) (i : Nat) : Outcome :=
  match m.nodes[i]? with
  | some (Node.atom _) => quoteReflect m defaultFuel i
  | none => .unsupported "quasiquote: node index out of range"
  | some (Node.list children) =>
    let hd := m.headName? (Node.list children)
    if hd == some "unquote".toUTF8 then
      match children[1]? with
      | some e =>
        if level ≤ 1 then
          -- ACTIVE unquote: evaluate + lift the value into its Ast representation.
          match evalNode m env defaultIntTy defaultFuel e with
          | .value v => valueToAst v
          | other => other
        else
          -- DEEPER unquote: decrement, keep the `(unquote …)` wrapper as structure.
          match evalQuasi m env (level - 1) e with
          | .value inner => .value (Value.variant "List".toUTF8 (Value.list
              #[Value.variant "Name".toUTF8 (Value.str "unquote".toUTF8), inner]))
          | other => other
      | none => .unsupported "quasiquote: unquote missing argument"
    else if hd == some "quasiquote".toUTF8 then
      match children[1]? with
      | some y =>
        -- nested quasiquote: increment, keep the `(quasiquote …)` wrapper as structure.
        match evalQuasi m env (level + 1) y with
        | .value inner => .value (Value.variant "List".toUTF8 (Value.list
            #[Value.variant "Name".toUTF8 (Value.str "quasiquote".toUTF8), inner]))
        | other => other
      | none => .unsupported "quasiquote: quasiquote missing argument"
    else if hd == some "unquote-splicing".toUTF8 then
      -- an `(unquote-splicing …)` splices a LIST's elements into its PARENT (handled per-child in the
      -- list fold below); a bare one that is itself the whole body is ill-formed → skip.
      .unsupported "quasiquote: unquote-splicing outside a list context"
    else
      -- ordinary (or headless) list: reflect each child at the same level into an Ast.List. A child
      -- `(unquote-splicing E)` at level 1 SPLICES E's list elements (each lifted) into this list,
      -- flattening; at a deeper level it decrements and stays a single `(unquote-splicing …)` node.
      let reflected : Except Outcome (Array Value) :=
        children.foldl (fun acc j =>
          match acc with
          | .error o => .error o
          | .ok vs =>
            match m.nodes[j]? with
            | some (Node.list jc) =>
              if m.headName? (Node.list jc) == some "unquote-splicing".toUTF8 then
                match jc[1]? with
                | some e =>
                  if level ≤ 1 then
                    match evalNode m env defaultIntTy defaultFuel e with
                    | .value (.list elems) =>
                      elems.foldl (fun a el =>
                        match a with
                        | .error o => .error o
                        | .ok vs2 =>
                          match valueToAst el with
                          | .value av => .ok (vs2.push av)
                          | other => .error other) (.ok vs)
                    | .value _ => .error (.unsupported "quasiquote: unquote-splicing of a non-list value")
                    | other => .error other
                  else
                    match evalQuasi m env (level - 1) e with
                    | .value inner => .ok (vs.push (Value.variant "List".toUTF8 (Value.list
                        #[Value.variant "Name".toUTF8 (Value.str "unquote-splicing".toUTF8), inner])))
                    | other => .error other
                | none => .error (.unsupported "quasiquote: unquote-splicing missing argument")
              else
                match evalQuasi m env level j with
                | .value v => .ok (vs.push v)
                | other => .error other
            | _ =>
              match evalQuasi m env level j with
              | .value v => .ok (vs.push v)
              | other => .error other) (.ok #[])
      match reflected with
      | .ok vs => .value (Value.variant "List".toUTF8 (Value.list vs))
      | .error o => o

/-- `(do stmt… lastExpr)` as an EXPRESSION (e.g. a function body): the LEADING statements are local
value bindings `(def name valueExpr)` bound SEQUENTIALLY + LAZILY (a later def sees the earlier), and the
FINAL child is the result expression, evaluated with all bindings in scope. A leading statement that is
NOT a `(def <bare-name> value)` — a local FUNCTION def `(def (f …) …)`, a bare effectful expression, an
`(export …)` — is not modeled → `unsupported` (a sound skip, never wrong semantics). -/
partial def evalDo (m : Module) (env : Env) (ty : IntTy) (fuel : Nat) (children : Array Nat) : Outcome :=
  let items := children.extract 1 children.size
  match items.back? with
  | none => .unsupported "eval: empty do"
  | some lastId =>
    let rec bindStmts (env : Env) (js : List Nat) : Except Outcome Env :=
      match js with
      | [] => .ok env
      | j :: rest =>
        match asDef? m j with
        | some dc =>
          match dc[1]?, dc[dc.size - 1]? with
          | some targetId, some valId =>
            match nameOf? m targetId with
            | some nm =>
              let bindTy := (operandTyEnv? m env valId).filter (fun t => t.width == .big)
              bindStmts ((nm, Thunk.mk (fun _ => evalNode m env defaultIntTy fuel valId), bindTy) :: env) rest
            | none =>
              -- a local FUNCTION def `(def (fname params) body)`: bind `fname` to a CLOSURE over the
              -- env-so-far (forced to values, like `evalFn`), so a later `(fname args)` call applies it.
              -- A self-/mutually-recursive local fn references a name absent from its captured env →
              -- unbound → skip (sound), since the capture is eager and doesn't include `fname` itself.
              match m.nodes[targetId]? with
              | some tnode =>
                match m.headName? tnode with
                | some fname =>
                  let params := paramSpecNodes m targetId
                  let cap := env.map (fun e => (e.1, outcomeToValue e.2.1.get))
                  bindStmts ((fname, Thunk.mk (fun _ => .value (Value.closure params valId cap)), none) :: env) rest
                | none => .error (.unsupported "eval: malformed do-block function def")
              | none => .error (.unsupported "eval: malformed do-block function def target")
          | _, _ => .error (.unsupported "eval: malformed do-block def")
        -- a NON-DEF statement: its VALUE is DISCARDED (not bound), so it is UNOBSERVED — its trap is ELIDED
        -- (core-semantics §"A Trap Occurs Only Where Its Computation Is Observed"; corpus 02-0385/0390,
        -- 14c-0745 "a discarded division NEVER traps the do"). So skip it and continue with the same env —
        -- a pure discarded statement contributes nothing to the value. (An effect perform's host side-effect
        -- is not modeled by the pure-value oracle either; such programs skip on the value-returning op.)
        | none => bindStmts env rest
    match bindStmts env (items.extract 0 (items.size - 1)).toList with
    | .ok env' => evalNode m env' ty fuel lastId
    | .error o => o

/-- Perform a Unit-returning effect op `((. eff op) args…)` — host-function modeling, H1a. The args flow
to the host, so they are OBSERVED (a trapping arg surfaces its trap); the op then yields `unit` (a
Unit-returning op's result is response-independent). The host CALL is not yet recorded (H3 will thread
the ordered call log through `execute`); only the response-independent VALUE is modeled here. -/
partial def performUnitOp (m : Module) (env : Env) (fuel : Nat) (children : Array Nat) : Outcome :=
  let argsOk : Outcome := (children.extract 1 children.size).foldl (fun (acc : Outcome) aid =>
    match acc with
    | .value _ => (match evalNode m env defaultIntTy fuel aid with
                   | .value v => observeDeep v
                   | other => other)
    | other => other) (Outcome.value Value.unit)
  match argsOk with | .value _ => Outcome.value Value.unit | other => other

/-- `(let (bindings) body)`: bind each `(name val)` SEQUENTIALLY (a later binding sees the earlier),
then evaluate `body`. Binding values are evaluated at the default integer type (their own annotation,
if any, sets it via `(: … )`); the body inherits the enclosing `ty`. -/
partial def evalLet (m : Module) (env : Env) (ty : IntTy) (fuel : Nat) (children : Array Nat) : Outcome :=
  match children[1]?, children[2]? with
  | some bindingsId, some bodyId =>
    match m.nodes[bindingsId]? with
    | some (Node.list pairs) =>
      -- Extend the env LAZILY: each binding a thunk capturing the env-so-far (sequential — a later
      -- binding sees the earlier). A binding is evaluated only when its variable is forced — EXCEPT a
      -- binding whose value contains a `?`/`try` (mentionsTry?), which is EAGER: the `?` short-circuit is
      -- control flow that fires at the binding even if the name is never forced. `.error` carries an
      -- Outcome — `.errReturn ev` short-circuits the whole `let` (→ the fn boundary), `.unsupported` a
      -- malformed binding.
      let rec extend (env : Env) (ps : List Nat) : Except Outcome Env :=
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
                -- propagate a BigInt-typed binding's width so later arithmetic on it stays unbounded
                -- (a `(let ((x (. BigInt of …))) (* x x))` must not false-overflow); other widths keep
                -- the Int64 default (narrowing widths are pending the trap-kind policy ruling). ENV-aware
                -- so a binding whose VALUE is arithmetic over PRIOR BigInt let-vars (`q = (/ n d)`) also
                -- infers BigInt — the chain that fixes the multi-limb division identity (06-numeric 0215/0255).
                let bindTy := (operandTyEnv? m env vId).filter (fun t => t.width == .big)
                let lazyThunk := Thunk.mk (fun _ => evalNode m captured defaultIntTy fuel vId)
                if rhsIsStrictCtor? m vId then
                  -- a list/set/map CONSTRUCTION binding is STRICT (ruling A, #5194/#5332): its element args
                  -- are forced at construction even when the binding is DISCARDED. Eager-eval and PROPAGATE
                  -- the outcome — a value binds; a trap/diverges/unsupported/errReturn propagates (the
                  -- construction WAS reached). Never fall back to lazy: that could return a wrong value for
                  -- a discarded ctor whose arg traps. A PURE clean ctor just binds the (unused) value.
                  match evalNode m captured defaultIntTy fuel vId with
                  | .value v => extend ((nm, (Thunk.mk (fun _ => .value v)), bindTy) :: env) rest
                  | other => .error other
                else if mentionsTry? m vId then
                  -- EAGER: fire the `?`. errReturn → short-circuit the let; a value → bind it (already
                  -- forced); a trap/diverges/unsupported → fall back to a LAZY thunk (do NOT force a pure
                  -- trap for an unused binding — only the `?` control flow needs eagerness).
                  match evalNode m captured defaultIntTy fuel vId with
                  | .errReturn ev => .error (.errReturn ev)
                  | .value v => extend ((nm, (Thunk.mk (fun _ => .value v)), bindTy) :: env) rest
                  | _ => extend ((nm, lazyThunk, bindTy) :: env) rest
                else
                  extend ((nm, lazyThunk, bindTy) :: env) rest
              | none => .error (.unsupported "eval: let binding target is not a name")
            | _, _ => .error (.unsupported "eval: malformed let binding pair")
          | _ => .error (.unsupported "eval: malformed let binding")
      match extend env pairs.toList with
      | .ok env' => evalNode m env' ty fuel bodyId
      | .error o => o
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
  | some valId, some tyId =>
    let o := evalNode m env ((parseIntTy? m tyId).getD ty) fuel valId
    let isF32 := (nameOf? m tyId) == some "Float32".toUTF8
    -- A `Rational` ascription GROUNDS a numeric LITERAL to its exact rational (06-numeric "Annotations
    -- Constrain"): a bare integer `n` → `n/1`; a decimal/scientific literal `±sig×10^exp` → the exact
    -- fraction (`exp≥0`: `sig·10^exp / 1`; `exp<0`: `sig / 10^-exp`), reduced+sign-normalized by mkRational
    -- (e.g. `0.5`→1/2, `0.1`→1/10, `-0.75`→-3/4, `12e2`→1200/1) — EXACT, no float rounding.
    let isRat := (nameOf? m tyId) == some "Rational".toUTF8
    match o with
    -- A `Float32` ascription over a COMPUTED float (`.f64`, an arithmetic result) can't be graded at f64
    -- precision: exact f32 arithmetic rounds at EACH op, and demoting only the final result would
    -- double-round. The evaluator doesn't yet thread float precision through the operations, so SKIP
    -- (Float32 ARITHMETIC is a pending increment). A Float64 (or unascribed) computed float grades normally.
    -- A COMPUTED f64 can't be grounded to an EXACT rational (its exact decimal is lost) → skip under Rational.
    | .value (.f64 _) => if isF32
                         then .unsupported "eval: Float32 arithmetic not modeled (per-op f32 demote pending)"
                         else if isRat
                         then .unsupported "eval: Rational grounding of a computed float not modeled"
                         else o
    -- A bare integer literal annotated Rational grounds to `n/1`.
    | .value (.int n) => if isRat then (match mkRational n 1 with | some v => .value v | none => o) else o
    -- A decimal/scientific literal annotated Rational grounds to its EXACT fraction (`sig×10^exp`).
    | .value (.float neg exp sig) =>
      if isRat then
        let mag0 : Int := Int.ofNat (Value.beBytesToNat sig)
        let mag : Int := if neg then -mag0 else mag0
        let p : Int := (10 : Int) ^ exp.natAbs
        (match (if exp ≥ 0 then mkRational (mag * p) 1 else mkRational mag p) with
         | some v => .value v | none => .trap "div-by-zero")
      -- A `Float32` ascription over a float LITERAL: DEMOTE to f32 precision (a SINGLE round — the literal's
      -- f64 value rounded to f32 and back), so two literals that share f32 bits compare equal.
      else if isF32 then
        (match Value.asF64? (.float neg exp sig) with
         | some f => .value (.f64 (Float.toFloat32 f).toFloat)
         | none => o)
      else o
    -- A `Float32` ascription over ±inf / NaN: DEMOTE to f32 precision (single round; ±inf/NaN are stable).
    | .value fv => if isF32 then
                     (match Value.asF64? fv with
                      | some f => .value (.f64 (Float.toFloat32 f).toFloat)
                      | none => o)
                   else o
    | _ => o
  | _, _ => .unsupported "eval: malformed ascription"

/-- `(op a b)` for a binary integer operator — evaluate both operands at `ty` and apply, trapping on
overflow / divide-by-zero per the width. Also handles the UNARY `(- e)` negation (spec: one-operand
subtraction = `0 - e` at the operand's type, so it traps on the MIN-value overflow / an unsigned
underflow exactly as `0 - e` does). -/
partial def evalArith (m : Module) (env : Env) (ty : IntTy) (fuel : Nat) (op : String) (children : Array Nat) : Outcome :=
  if op == "-" && children.size == 2 then
    match children[1]? with
    | some eId =>
      let opTy := (operandTyEnv? m env eId).getD ty
      match evalNode m env opTy fuel eId with
      | .value (.int a) => evalArithOp "-" 0 a opTy (overflowWraps? m opTy.signed)  -- negation = 0 - a at the operand's width
      | .value _ => .unsupported "eval: unary minus of a non-integer"
      | other => other
    | none => .unsupported "eval: malformed unary minus"
  else match children[1]?, children[2]? with
  | some aId, some bId =>
    if children.size != 3 then .unsupported s!"eval: {op} expects 2 operands"
    else
      -- the op's width comes from an operand's ascription OR a bound (param) variable's declared
      -- type, if either is present; else the ambient type
      let operandTyIn := fun (i : Nat) => operandTyEnv? m env i
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
      | .errReturn v, _ => .errReturn v
      | _, .errReturn v => .errReturn v
      | .value va, .value vb =>
        match va, vb with
        | .int a, .int b => evalArithOp op a b opTy (overflowWraps? m opTy.signed)
        -- exact Rational arithmetic (`+ - * /`): closed, normalized, no rounding; `/` by 0 traps.
        | .rational a b, .rational c d => rationalArith op a b c d
        | _, _ =>
          -- both operands are floats (a literal, ±inf/NaN, or a computed f64) → IEEE float arithmetic.
          -- Skip if a Float32 is involved (per-op f32 rounding not yet threaded → f64 compute would be wrong).
          match Value.asF64? va, Value.asF64? vb with
          | some fa, some fb =>
            if mentionsFloat32? m aId || mentionsFloat32? m bId
            then .unsupported "eval: Float32 arithmetic not modeled (per-op f32 demote pending)"
            else evalFloatOp op fa fb
          | _, _ => .unsupported "eval: non-numeric operand to arithmetic"
  | _, _ => .unsupported s!"eval: malformed {op}"

/-- `(op a b)` for a binary bitwise / shift operator — same operand evaluation + width inference as
`evalArith` (precedence unsupported > diverges > trap > value), then apply `evalBitOp`. -/
partial def evalBitwise (m : Module) (env : Env) (ty : IntTy) (fuel : Nat) (op : String) (children : Array Nat) : Outcome :=
  match children[1]?, children[2]? with
  | some aId, some bId =>
    if children.size != 3 then .unsupported s!"eval: {op} expects 2 operands"
    else
      let operandTyIn := fun (i : Nat) => operandTyEnv? m env i
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
      | .errReturn v, _ => .errReturn v
      | _, .errReturn v => .errReturn v
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

/-- `(try E)` (the `?` operator): evaluate the fallible operand `E` and either UNWRAP its success payload
or SHORT-CIRCUIT the enclosing fallible function with its failure (§4 boundary). `Ok v`/`Some v` → `v`;
`Err e` → `.errReturn (Err e)`; `None` → `.errReturn None` — the errReturn propagates up to the function
boundary (evalCall/applyClosure/execute), which turns it into the function's `.value`. The unwrapped/failed
payload stays as-is (a lazy `poison` payload is not forced here — `?` inspects only the Ok/Err discriminant).
A non-Option/Result operand is a type error the compiler rejects (sound skip); the operand's own
trap/diverges/errReturn propagates. -/
partial def evalTry (m : Module) (env : Env) (fuel : Nat) (children : Array Nat) : Outcome :=
  match children[1]? with
  | some eId =>
    match evalNode m env defaultIntTy fuel eId with
    | .value (.ok v) => .value v
    | .value (.some v) => .value v
    | .value (.err e) => .errReturn (.err e)
    | .value .none => .errReturn .none
    | .value _ => .unsupported "eval: try operand is not an Option/Result value"
    | other => other
  | none => .unsupported "eval: malformed try"

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
  -- DECREMENT fuel per call so an unbounded/too-deep recursion yields `diverges` instead of HANGING
  -- (a genuine infinite/large loop consumes fuel down to 0 — a sound skip, never a wedged process).
  match fuel with
  | 0 => .diverges
  | Nat.succ fuel' =>
    let args := children.extract 1 children.size
    let bindings := (paramSpecs.zip args).filterMap (fun (specId, argId) =>
      (paramSpec? m specId).map (fun (nm, ty) => (nm, (Thunk.mk (fun _ => evalNode m env (ty.getD defaultIntTy) fuel' argId)), ty)))
    if bindings.size == paramSpecs.size then
      -- FUNCTION BOUNDARY: a `?`/`try` short-circuit (errReturn) from the body becomes this call's value.
      (match evalNode m bindings.toList defaultIntTy fuel' bodyId with
       | .errReturn ev => .value ev
       | o => o)
    else .unsupported "eval: call has a malformed parameter spec"

/-- Expand a call's raw argument nodes into positional `ArgSrc`s, spreading any splat `(.. e)` argument:
`e` is evaluated STRICTLY (the tuple/list must be built to spread it — matching the compiler's per-slot
projection), and a tuple/list value contributes its elements as separate positional args; a non-tuple/list
splat operand is a sound skip, and a trap/diverges/errReturn from the operand propagates as the call's
outcome. A non-splat arg passes through as a lazy `.node` (an unused parameter's arg is never forced). -/
partial def expandArgs (m : Module) (env : Env) (fuel : Nat) (args : Array Nat) : Except Outcome (Array ArgSrc) := do
  let mut out : Array ArgSrc := #[]
  for aid in args do
    match splatOperand? m aid with
    | some opId =>
      match evalNode m env defaultIntTy fuel opId with
      | .value (.tuple es) => out := out ++ es.map ArgSrc.val
      | .value (.list es)  => out := out ++ es.map ArgSrc.val
      | .value _ => throw (.unsupported "eval: splat operand is not a tuple or list")
      | o => throw o
    | none => out := out.push (ArgSrc.node aid)
  return out

/-- A call `(f arg… (.. tuple) arg…)` carrying at least one SPLAT argument: expand the args (spreading each
splat's tuple/list elements into positional slots), then — if the expanded count matches `f`'s arity —
bind each slot under its parameter (a `.node` slot lazily over the CALLER's env, a splat-produced `.val`
slot as the already-computed value) and evaluate the body. A non-matching expanded arity (partial/over-
application via splat) is not modeled → a sound skip. -/
partial def evalCallSplat (m : Module) (env : Env) (fuel : Nat) (paramSpecs : Array Nat) (bodyId : Nat) (children : Array Nat) : Outcome :=
  match fuel with
  | 0 => .diverges
  | Nat.succ fuel' =>
    match expandArgs m env fuel' (children.extract 1 children.size) with
    | .error o => o
    | .ok srcs =>
      if srcs.size != paramSpecs.size then
        .unsupported "eval: splat-expanded call arity mismatch (partial/over-application via splat not modeled)"
      else
        let bindings := (paramSpecs.zip srcs).filterMap (fun (specId, src) =>
          (paramSpec? m specId).map (fun (nm, ty) =>
            (nm, Thunk.mk (fun _ => match src with
                                    | .node aid => evalNode m env (ty.getD defaultIntTy) fuel' aid
                                    | .val v => Outcome.value v), ty)))
        if bindings.size == paramSpecs.size then
          (match evalNode m bindings.toList defaultIntTy fuel' bodyId with
           | .errReturn ev => .value ev
           | o => o)
        else .unsupported "eval: call has a malformed parameter spec"

/-- `(fn (param…) body)` → a closure value capturing the CURRENT env (each binding forced now to a value
or a `poison`, so an unused captured binding never surfaces its trap; laziness preserved via poison). -/
partial def evalFn (m : Module) (env : Env) (fuel : Nat) (children : Array Nat) : Outcome :=
  match children[1]?, children[2]? with
  | some paramListId, some bodyId =>
    let params := match m.nodes[paramListId]? with | some (Node.list ps) => ps | _ => #[]
    let cap := env.map (fun e => (e.1, outcomeToValue e.2.1.get))
    .value (.closure params bodyId cap)
  | _, _ => .unsupported "eval: malformed fn"

/-- Apply a closure to a FULLY-supplied argument list: bind each arg LAZILY (over the caller's env) under
its parameter name + declared type, plus the captured env (each name → its stored value, observed
shallowly on use), then evaluate the body. A partial application (wrong arg count) is not modeled → skip. -/
partial def applyClosure (m : Module) (env : Env) (fuel : Nat) (params : Array Nat) (body : Nat)
    (cap : List (ByteArray × Value)) (children : Array Nat) : Outcome :=
  -- DECREMENT fuel per application (like evalCall) so a recursive closure diverges rather than hangs.
  match fuel with
  | 0 => .diverges
  | Nat.succ fuel' =>
    let args := children.extract 1 children.size
    if args.size < params.size && 0 < args.size then
      -- PARTIAL application of a closure: capture the given args (values) under their param names, plus the
      -- existing cap, into a NEW closure over the remaining params (currying — `((add3 1) 2)` → a 1-ary).
      let newCap := cap ++ ((params.extract 0 args.size).zip args).toList.filterMap (fun (sp, aid) =>
        (paramSpec? m sp).map (fun (nm, _) => (nm, outcomeToValue (evalNode m env defaultIntTy fuel' aid))))
      .value (.closure (params.extract args.size params.size) body newCap)
    else if params.size != args.size then .unsupported "eval: closure arity mismatch (over-application not modeled)"
    else
      let argBindings : Env := (params.zip args).toList.filterMap (fun (specId, argId) =>
        (paramSpec? m specId).map (fun (nm, ty) => (nm, (Thunk.mk (fun _ => evalNode m env (ty.getD defaultIntTy) fuel' argId)), ty)))
      let capBindings : Env := cap.map (fun (nm, v) => (nm, (Thunk.mk (fun _ => observeShallow v)), Option.none))
      if argBindings.length == params.size then
        -- FUNCTION BOUNDARY: a `?`/`try` short-circuit from the closure body becomes the application's value.
        (match evalNode m (argBindings ++ capBindings) defaultIntTy fuel' body with
         | .errReturn ev => .value ev
         | o => o)
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
  else if is "Set" "len" then
    some (match a1 with | some (.value (.set es)) => .value (.int es.size)
                        | some (.value _) => .unsupported "Set.len: not a set" | some o => o | none => .unsupported "Set.len arity")
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
  else if is "Rational" "of" then
    -- `(Rational.of n d)` → the normalized exact rational `n/d`. A ZERO denominator fails the CHECKED
    -- rational construction and traps kind `unreachable` (NOT `divide by zero`, which is for integer
    -- `/`/`%`) — like the other checked `.of` conversions. Pinned by corpus 06-numeric-model/0080 +
    -- 28-compiler-primitives (`List.len` over `(Rational.of 3 0)`) + 03-equality dead-let list, all
    -- `(trap "unreachable")`, and 02-binding-and-control ("zero-denominator Rational.of" trap kind).
    some (match a1, a2 with
          | some (.value (.int n)), some (.value (.int d)) =>
            (match mkRational n d with | some v => .value v | none => .trap "unreachable")
          | some (.unsupported r), _ | _, some (.unsupported r) => .unsupported r
          | some (.trap t), _ | _, some (.trap t) => .trap t
          | some .diverges, _ | _, some .diverges => .diverges
          | _, _ => .unsupported "Rational.of: non-integer operand")
  else if is "Set" "contains" then
    some (match a1, a2 with
          | some (.value (.set es)), some (.value x) => .value (.bool (es.any (valEq · x)))
          | some (.unsupported r), _ | _, some (.unsupported r) => .unsupported r
          | some (.trap t), _ | _, some (.trap t) => .trap t
          | some .diverges, _ | _, some .diverges => .diverges
          | _, _ => .unsupported "Set.contains: operand")
  else if is "Set" "insert" then
    -- add an element, re-canonicalize (sort + dedupe) — the Set twin of Map.insert.
    some (match a1, a2 with
          | some (.value (.set es)), some (.value x) =>
            (match canonSet (es.push x) with | some s => .value (.set s) | none => .unsupported "Set.insert: unorderable element")
          | some (.unsupported r), _ | _, some (.unsupported r) => .unsupported r
          | some (.trap t), _ | _, some (.trap t) => .trap t
          | some .diverges, _ | _, some .diverges => .diverges
          | _, _ => .unsupported "Set.insert: operand")
  else if is "Map" "lookup" then
    some (match a1, a2 with
          | some (.value (.map es)), some (.value k) =>
            (match (es.find? (fun kv => valEq kv.1 k)).map (·.2) with | some v => .value (.some v) | none => .value .none)
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
  else if is "String" "slice" then
    -- SCALAR-indexed substring `String.slice s start end` → `Some s[start..end)` when
    -- `0 ≤ start ≤ end ≤ scalar-count`, else `None` (fallible VIEW; 13-strings §227/1428). Three operands.
    let a3 := (children[3]?).map (fun i => evalNode m env defaultIntTy fuel i)
    some (match a1, a2, a3 with
          | some (.value (.str bytes)), some (.value (.int start)), some (.value (.int «end») ) =>
            (match String.fromUTF8? bytes with
             | some s =>
               let cs := s.toList
               if 0 ≤ start && start ≤ «end» && «end» ≤ Int.ofNat cs.length then
                 .value (.some (.str (String.toUTF8 (String.mk ((cs.drop start.toNat).take («end».toNat - start.toNat))))))
               else .value .none
             | none => .unsupported "String.slice: invalid UTF-8")
          | some (.unsupported r), _, _ | _, some (.unsupported r), _ | _, _, some (.unsupported r) => .unsupported r
          | some (.trap t), _, _ | _, some (.trap t), _ | _, _, some (.trap t) => .trap t
          | some .diverges, _, _ | _, some .diverges, _ | _, _, some .diverges => .diverges
          | _, _, _ => .unsupported "String.slice: operand")
  else if is "String" "at" then
    -- indexed CHARACTER access (by Unicode SCALAR, matching Lean's String.data) → Option single-char
    -- String: `Some s[i]` when 0 ≤ i < char-count, else `None`. (`"café"[3]="é"`, `"😀b"[1]="b"`.)
    some (match a1, a2 with
          | some (.value (.str bytes)), some (.value (.int i)) =>
            (match String.fromUTF8? bytes with
             | some s => let cs := s.toList
                         if 0 ≤ i && i < Int.ofNat cs.length then .value (.some (.str (String.toUTF8 (cs[i.toNat]!).toString)))
                         else .value .none
             | none => .unsupported "String.at: invalid UTF-8")
          | some (.unsupported r), _ | _, some (.unsupported r) => .unsupported r
          | some (.trap t), _ | _, some (.trap t) => .trap t
          | some .diverges, _ | _, some .diverges => .diverges
          | _, _ => .unsupported "String.at: operand")
  else if is "String" "scalar-at" then
    -- SCALAR-indexed access → Option<Char> (spec collections-and-text §"A String's Scalars Are
    -- Addressable": `(String.scalar-at "hello" 1)` = `Some #\e`). Same Unicode-scalar indexing as
    -- `String.at`, but the read yields the CHAR value (`.char`), not a single-char string.
    some (match a1, a2 with
          | some (.value (.str bytes)), some (.value (.int i)) =>
            (match String.fromUTF8? bytes with
             | some s => let cs := s.toList
                         if 0 ≤ i && i < Int.ofNat cs.length then .value (.some (.char (String.toUTF8 (cs[i.toNat]!).toString)))
                         else .value .none
             | none => .unsupported "String.scalar-at: invalid UTF-8")
          | some (.unsupported r), _ | _, some (.unsupported r) => .unsupported r
          | some (.trap t), _ | _, some (.trap t) => .trap t
          | some .diverges, _ | _, some .diverges => .diverges
          | _, _ => .unsupported "String.scalar-at: operand")
  else if is "Bytes" "len" then
    some (match a1 with | some (.value (.bytes b)) => .value (.int (Int.ofNat b.size))
                        | some (.value _) => .unsupported "Bytes.len: not a bytes" | some o => o | none => .unsupported "Bytes.len arity")
  else if is "Bytes" "slice" then
    -- `Bytes.slice b start LENGTH` → `Some b[start .. start+length)` (byte-indexed, start/LENGTH — NOT
    -- start/end like String.slice) when `0 ≤ start`, `0 ≤ length`, `start+length ≤ len`, else `None`
    -- (fallible VIEW; 10-bytes §172/185: `(Bytes.slice [10 20 30 40] 1 2)` = Some [20 30]). Three operands.
    let a3 := (children[3]?).map (fun i => evalNode m env defaultIntTy fuel i)
    some (match a1, a2, a3 with
          | some (.value (.bytes b)), some (.value (.int start)), some (.value (.int len)) =>
            if 0 ≤ start && 0 ≤ len && start + len ≤ Int.ofNat b.size then
              .value (.some (.bytes (b.extract start.toNat (start.toNat + len.toNat))))
            else .value .none
          | some (.unsupported r), _, _ | _, some (.unsupported r), _ | _, _, some (.unsupported r) => .unsupported r
          | some (.trap t), _, _ | _, some (.trap t), _ | _, _, some (.trap t) => .trap t
          | some .diverges, _, _ | _, some .diverges, _ | _, _, some .diverges => .diverges
          | _, _, _ => .unsupported "Bytes.slice: operand")
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
            (match canonMap (es.filter (fun kv => !(valEq kv.1 k))) with | some cm => .value (.map cm) | none => .unsupported "Map.remove: unorderable key")
          | some (.unsupported r), _ | _, some (.unsupported r) => .unsupported r
          | some (.trap t), _ | _, some (.trap t) => .trap t
          | some .diverges, _ | _, some .diverges => .diverges
          | _, _ => .unsupported "Map.remove: operand")
  else if is "String" "concat" then
    some (match a1, a2 with
          | some (.value (.str x)), some (.value (.str y)) => .value (.str (x ++ y))
          | some (.unsupported r), _ | _, some (.unsupported r) => .unsupported r
          | some (.trap t), _ | _, some (.trap t) => .trap t
          | some .diverges, _ | _, some .diverges => .diverges
          | _, _ => .unsupported "String.concat: non-string operand")
  else if is "String" "byte-len" then
    some (match a1 with | some (.value (.str b)) => .value (.int (Int.ofNat b.size))
                        | some (.value _) => .unsupported "String.byte-len: not a string" | some o => o | none => .unsupported "arity")
  else if is "String" "scalar-len" then
    some (match a1 with
          | some (.value (.str b)) => (match String.fromUTF8? b with | some s => .value (.int (Int.ofNat s.toList.length)) | none => .unsupported "String.scalar-len: invalid UTF-8")
          | some (.value _) => .unsupported "String.scalar-len: not a string" | some o => o | none => .unsupported "arity")
  else if (parseIntTyName? qual).isSome && (mem == "wrapping-add".toUTF8 || mem == "wrapping-sub".toUTF8 || mem == "wrapping-mul".toUTF8) then
    -- WRAPPING arithmetic: (x op y) reduced mod 2^w (total, never traps — the non-trapping companion of +/-/*).
    let tty := (parseIntTyName? qual).get!
    some (match a1, a2 with
          | some (.value (.int x)), some (.value (.int y)) =>
            let r := if mem == "wrapping-add".toUTF8 then x + y else if mem == "wrapping-sub".toUTF8 then x - y else x * y
            (match tty.width with
             | .bits w => let modw : Int := (2 : Int) ^ w
                          let p := ((r % modw) + modw) % modw
                          .value (.int (if tty.signed && p ≥ (2 : Int) ^ (w - 1) then p - modw else p))
             | _ => .value (.int r))       -- BigInt: no wrapping (unbounded)
          | some (.unsupported r), _ | _, some (.unsupported r) => .unsupported r
          | some (.trap t), _ | _, some (.trap t) => .trap t
          | some .diverges, _ | _, some .diverges => .diverges
          | _, _ => .unsupported "wrapping arithmetic: non-integer operand")
  else if (parseIntTyName? qual).isSome && (mem == "wrap".toUTF8 || mem == "of".toUTF8) then
    -- numeric conversion `(. <IntTy> wrap|of) x`: `wrap` reinterprets x mod 2^w (total); `of` is CHECKED —
    -- it range-checks and, if x is out of the target range, TRAPS with kind `unreachable` (the range-check
    -- `if x∉[lo,hi] then <unreachable>`), NOT `overflow` (which is for arithmetic ops). Pinned by corpus
    -- 06-numeric: `Int8.of (200:UInt8)`, `Int64.of (UInt8.of n)`, `Rational.truncate` R→i64 overflow all
    -- `(trap "unreachable")` (:4088/:4106/:343 — "wasm traps unreachable"). On BigInt both are identity.
    -- Value = int (type stripped by the grader). Wrap-vs-of chosen per the member name.
    let tty := (parseIntTyName? qual).get!
    some (match a1 with
          | some (.value (.int x)) =>
            (match tty.width with
             | .bits w =>
               let modw : Int := (2 : Int) ^ w
               if mem == "wrap".toUTF8 then
                 let p := ((x % modw) + modw) % modw
                 .value (.int (if tty.signed && p ≥ (2 : Int) ^ (w - 1) then p - modw else p))
               else
                 let lo : Int := if tty.signed then -((2 : Int) ^ (w - 1)) else 0
                 let hi : Int := if tty.signed then (2 : Int) ^ (w - 1) else (2 : Int) ^ w
                 if lo ≤ x && x < hi then .value (.int x) else .trap "unreachable"
             | _ => .value (.int x))            -- BigInt: identity (both wrap and of)
          | some (.value _) => .unsupported "numeric conversion: non-integer operand"
          | some o => o | none => .unsupported "numeric conversion arity")
  else none

/-- A generic sum constructor application `(C …)` / `((. T C) …)`: nullary → `variant C unit`; single-field
→ `variant C payload` (payload deferred as a `poison` if non-value, like a tuple/record field — spec Q2). -/
partial def evalVariantCtor (m : Module) (env : Env) (fuel : Nat) (cname : ByteArray) (arity : Nat) (children : Array Nat) : Outcome :=
  -- UNIFY the prelude Option/Result ctor NAMES with the built-in Value: a `Some`/`Ok`/`Err`/`None`
  -- ctor — whether reached bare, qualified `(. Option Some)`, or scanned from a user `(type Option …)`
  -- that redeclares the prelude sum — produces the SAME `Value.some`/`ok`/`err`/`none` the checker's
  -- `expectedValue?` parses `(Some x)` etc. into. A sum value's form `(Some x)` is one value regardless
  -- of how the ctor was named/declared, and the grader strips the type — so this keeps construction and
  -- the expected value-form in one representation (fixes user-redeclared Option/Result escapes: 07-0041/0042).
  let cs := (String.fromUTF8? cname).getD ""
  let payload : Unit → Value := fun _ => match children[1]? with
    | some pId => outcomeToValue (evalNode m env defaultIntTy fuel pId) | none => Value.unit
  -- Guard on the CANONICAL arity (None nullary; Some/Ok/Err single-field): a user type that merely
  -- reuses one of these names at a DIFFERENT arity is a distinct ctor → fall through to the generic
  -- `variant` path rather than mis-unwrapping it into the built-in Option/Result value.
  if cs == "None" && arity == 0 then .value Value.none
  else if cs == "Some" && arity == 1 then .value (Value.some (payload ()))
  else if cs == "Ok" && arity == 1 then .value (Value.ok (payload ()))
  else if cs == "Err" && arity == 1 then .value (Value.err (payload ()))
  else
    -- the field arguments (children after the ctor head), each stored as a value or a deferred `poison`
    -- (lazy, like a tuple/record field — spec Q2).
    let fields := (children.extract 1 children.size).map (fun pId => outcomeToValue (evalNode m env defaultIntTy fuel pId))
    match arity with
    -- a SOLE nullary ctor erases to `unit` (single-ctor type carries no info); a nullary ctor of a
    -- MULTI-ctor type stays a tagged variant (needed to tell it from its siblings).
    | 0 => .value (if soleNullaryCtor? m cname then .unit else .variant cname .unit)
    -- single-field → `variant C payload`; MULTI-field (≥2) → `variant C (tuple f1…fN)` (the payload is a
    -- tuple of the fields — symmetric with matchPat's `(C p1…pN)` and expectedValue?'s `(C v1…vN)`).
    | 1 => .value (.variant cname (fields[0]?.getD .unit))
    | _ => .value (.variant cname (.tuple fields))

/-- A sequence constructor `(tuple e…)` / `(list e…)`: evaluate each element, storing a non-value
element as a `poison` (deferred) rather than propagating it — an element that is never observed
(projected, or flowed to the result) never surfaces its trap. Construction itself always yields a
value. -/
partial def evalSeqCtor (m : Module) (env : Env) (fuel : Nat) (children : Array Nat)
    (wrap : Array Value → Value) (strict : Bool) : Outcome :=
  -- STRICT (list): a list is heap-MATERIALIZED, so construction FORCES each element — a non-value outcome
  -- (trap/unsupported/diverges) PROPAGATES, i.e. a trapping element traps at construction even if only the
  -- length is later taken (operator ruling A / #5194; corpus 28-compiler-primitives:2411, 06-numeric/0043).
  -- The force is DEEP (observeDeep): the list's heap cells hold VALUES not thunks, so a DEFERRED trap in a
  -- materialized tuple/record SLOT surfaces too — `(list (tuple (/ 5 0) 1) …)` traps even though the tuple's
  -- slot 0 is a deferred poison and the tuple is later dropped (breaker #5227, 06-numeric case 1108). A
  -- STANDALONE tuple stays lazy (#5145 eq); it is materializing INTO the heap list that forces the slots.
  -- NON-strict (tuple/record): store a non-value element as a `poison` (lazy — unobserved until projected).
  let rec go (js : List Nat) (acc : Except Outcome (Array Value)) : Except Outcome (Array Value) :=
    match js with
    | [] => acc
    | j :: rest =>
      match acc with
      | .error o => .error o
      | .ok vs =>
        if strict then
          match evalNode m env defaultIntTy fuel j with
          | .value v => (match observeDeep v with
                         | .value fv => go rest (.ok (vs.push fv))
                         | forced => .error forced)   -- a deferred inner slot trap surfaces at materialization
          | other => .error other
        else go rest (.ok (vs.push (outcomeToValue (evalNode m env defaultIntTy fuel j))))
  match go (children.extract 1 children.size).toList (.ok #[]) with
  | .ok vs => .value (wrap vs)
  | .error o => o

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
  | .errReturn v, _ => .errReturn v
  | _, .errReturn v => .errReturn v
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
    else
      -- Evaluate each operand, then compare. STRICT list construction (operator ruling A, corpus #5194):
      -- a `(list …)` operand is materialized by the DEFAULT evalNode (evalSeqCtor strict), so every element
      -- ARGUMENT is evaluated at construction — a trapping arg traps BEFORE `=` runs, in ANY consumer,
      -- independent of comparison short-circuit (`(= (list 9 (/ 5 0)) …)` and `(= (list 1 (/ 5 0)) …)` both
      -- TRAP at d=0). `eqSC` may still short-circuit WHICH already-evaluated VALUES it inspects (a tuple's
      -- unprojected element stays a lazy poison — tuples/records are lazy, #5145 — while a list's elements
      -- are all forced), but it never DEFERS a constructed operand's argument evaluation. (This reverts the
      -- earlier list-`=` short-circuit #5176; the operator ruled A STRICT over the B lean.)
      match evalNode m env defaultIntTy fuel aId with
      | .value va =>
        (match evalNode m env defaultIntTy fuel bId with
         | .value vb => eqSC va vb
         | other => other)
      | other => other
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
      | none =>
        -- FLOAT ordering (`<`/`>`/`<=`/`>=`): IEEE partial order — Lean's `Float` comparison is IEEE, so a
        -- NaN operand makes every comparison FALSE, and `-0.0`/`+0.0` compare equal. (A Float32-ascribed
        -- operand is already demoted to its f32 value by evalAscribe, so it orders on the f32 bits.)
        match Value.asF64? va, Value.asF64? vb with
        | some fa, some fb =>
          .value (.bool (match op with
                         | "<" => fa < fb | ">" => fa > fb | "<=" => fa ≤ fb | ">=" => fa ≥ fb | _ => false))
        | _, _ => .unsupported "eval: ordering on a type that offers no total order (compound/unit)")
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

/-- `(set e…)` — a native SET literal (`#set(…)`): evaluate the elements STRICTLY (a set is heap-
materialized — ruling A/#5150 forces element args, deeply via `evalSeqCtor`), then canonicalize (sort +
dedupe) into a `Value.set`. A trapping element traps at construction; an unorderable element (compound/
poison) → skip. Same `Value.set` the checker's `expectedValue?` `"set"` branch and `evalSetOf` produce. -/
partial def evalSetLiteral (m : Module) (env : Env) (fuel : Nat) (children : Array Nat) : Outcome :=
  match evalSeqCtor m env fuel children Value.list true with
  | .value (.list es) => match canonSet es with
                         | some s => .value (.set s)
                         | none => .unsupported "eval: set element is unorderable (compound/poison)"
  | other => other

/-- `(map (k1 v1) (k2 v2)…)` — a map LITERAL: evaluate each `(k v)` entry to a key/value pair, then
canonicalize with `canonMap` (sort by key, dedupe; an unorderable key → skip). Mirrors the `(map (k v)…)`
value form the checker's `expectedValue?` reads, and is the construction dual of the `(map (k p)…)`
match pattern. A non-value key/value propagates its outcome (a map key must be forced to be ordered). -/
partial def evalMapLiteral (m : Module) (env : Env) (fuel : Nat) (children : Array Nat) : Outcome :=
  let rec go (js : List Nat) (acc : Array (Value × Value)) : Except Outcome (Array (Value × Value)) :=
    match js with
    | [] => .ok acc
    | j :: rest =>
      match m.nodes[j]? with
      | some (Node.list ec) =>
        let (kId, vId) := match m.headName? (Node.list ec) with
          | some h => if h == "=".toUTF8 && ec.size == 3 then (ec[1]?, ec[2]?) else (ec[0]?, ec[1]?)
          | none => (ec[0]?, ec[1]?)
        match kId, vId with
        | some kId, some vId =>
          (match evalNode m env defaultIntTy fuel kId with
           | .value kv => (match evalNode m env defaultIntTy fuel vId with
                           | .value vv => go rest (acc.push (kv, vv))
                           | other => .error other)
           | other => .error other)
        | _, _ => .error (.unsupported "eval: malformed map entry")
      | _ => .error (.unsupported "eval: malformed map entry")
  match go (children.extract 1 children.size).toList #[] with
  | .ok entries => (match canonMap entries with
                    | some c => .value (.map c)
                    | none => .unsupported "eval: map literal has an unorderable key")
  | .error o => o

/-- `(. recExpr field)` — project a named field from a record value (spec §Member Access): observe the
projected field SHALLOWLY (its top-level poison surfaces; a nested compound stays lazy). A non-record
operand or a non-name field key (tuple positional access, etc.) is not modeled → skip. -/
partial def evalProject (m : Module) (env : Env) (fuel : Nat) (children : Array Nat) : Outcome :=
  match children[1]?, children[2]? with
  | some recId, some fieldId =>
    -- `(. Map empty)` used as a value = the empty map (a prelude module value, not a record projection).
    if (nameOf? m recId == some "Map".toUTF8) && (nameOf? m fieldId == some "empty".toUTF8) then .value (.map #[])
    -- prelude float CONSTANTS `Float64.nan` / `Float32.nan` (a member-access value, not a projection) →
    -- the canonical NaN (all NaN spellings unify via specFloatEq / valEq's canonical-bits order).
    else if ((nameOf? m recId == some "Float64".toUTF8) || (nameOf? m recId == some "Float32".toUTF8))
            && (nameOf? m fieldId == some "nan".toUTF8) then .value .floatNan
    -- prelude float constant `Float64.Infinity` / `Float32.Infinity` → +∞ (negative infinity is `(- …)`).
    else if ((nameOf? m recId == some "Float64".toUTF8) || (nameOf? m recId == some "Float32".toUTF8))
            && (nameOf? m fieldId == some "Infinity".toUTF8) then .value (.floatInf false)
    else
    match evalNode m env defaultIntTy fuel recId with
    | .value (.record fields) =>
      match nameOf? m fieldId with
      | some key =>
        match (fields.find? (fun kv => kv.1 == key)).map (·.2) with
        | some fv => observeShallow fv
        | none => .unsupported "eval: record has no such field (typecheck not modeled)"
      | none => .unsupported "eval: projection key is not a field name"
    -- positional TUPLE access `(. tup i)` — the field is an INTEGER index → the i-th element (observed
    -- shallowly, like a record field). An out-of-arity index is a COMPILE error (CDZ0201, not a runtime
    -- trap — a tuple's arity is static), so those cases are `expect-error` skips; a runtime out-of-range
    -- reaching here is an unmodeled shape → sound skip.
    | .value (.tuple es) =>
      match (m.nodes[fieldId]?).bind (fun n => match n with
              | Node.atom lid => (m.leaves[lid]?).bind Value.ofLeaf | _ => none) with
      | some (.int i) => if 0 ≤ i && i < Int.ofNat es.size then observeShallow (es[i.toNat]!)
                         else .unsupported "eval: tuple index out of arity (compile error, not modeled)"
      | _ => .unsupported "eval: tuple projection index is not an integer literal"
    | .value _ => .unsupported "eval: projection operand is not a record/tuple (other access not modeled)"
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
      else .ok (some [(b, (Thunk.mk (fun _ => observeShallow subj)), Option.none)])
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
      (match ctorAppName? m pc with
       | some cname =>
         if newtypeCtor? m cname then some (match pc[1]? with | some sp => matchPat m sp subj | none => .ok (some []))
         -- a struct-newtype pattern `(Mk p1…pN)` erases: the subject IS the bare TUPLE of fields → match
         -- each subpattern against its tuple element (no tag to compare).
         else if structNewtypeCtor? m cname then
           some (let subpats := pc.extract 1 pc.size
                 match subj with
                 | .tuple es => if es.size == subpats.size then matchSeq m (subpats.zip es).toList else .ok none
                 | _ => .ok none)
         else match variantCtorArity? m cname with
           | some ar =>
             -- SYMMETRIC with evalVariantCtor: a user ctor named None/Some/Ok/Err at the CANONICAL arity
             -- constructs the built-in Value.none/some/ok/err, so its pattern must match that value form
             -- (not a tagged `variant`); any other name/arity stays a tagged-variant pattern.
             let csm := (String.fromUTF8? cname).getD ""
             some (
               if csm == "None" && ar == 0 then (match subj with | .none => .ok (some []) | _ => .ok none)
               else if csm == "Some" && ar == 1 then (match subj, pc[1]? with | .some p, some sp => matchPat m sp p | .some _, none => .ok (some []) | _, _ => .ok none)
               else if csm == "Ok" && ar == 1 then (match subj, pc[1]? with | .ok p, some sp => matchPat m sp p | .ok _, none => .ok (some []) | _, _ => .ok none)
               else if csm == "Err" && ar == 1 then (match subj, pc[1]? with | .err p, some sp => matchPat m sp p | .err _, none => .ok (some []) | _, _ => .ok none)
               else match subj with
                    | .variant tag payload =>
                      if tag == cname then
                        -- 0 subpats → nullary; 1 → match the single payload; ≥2 → the payload is a TUPLE
                        -- of the fields (symmetric with the multi-field ctor construction), match each.
                        let subpats := pc.extract 1 pc.size
                        (match subpats.size with
                         | 0 => .ok (some [])
                         | 1 => matchPat m subpats[0]! payload
                         | _ => (match payload with
                                 | .tuple es => if es.size == subpats.size then matchSeq m (subpats.zip es).toList else .ok none
                                 | _ => .ok none))
                      else .ok none
                    | _ => .ok none)
           | none => none
       | none => none)
      <|>
      -- a prelude `Ast` variant pattern `((. Ast Ctor) subpat)` (Int/Float/Name/…): the `Ast` sum is
      -- built-in (not in the scanned `(type …)` decls), so recognize its qualified ctors directly and
      -- match the tagged `variant` value that `quote`/`eval` produce, binding the subpattern to the payload.
      (match qualHead? m pc with
       | some (q, c) =>
         if q == "Ast".toUTF8 &&
            ["Int", "Float", "Bool", "Str", "Name", "List", "Bytes", "Char", "Symbol"].contains ((String.fromUTF8? c).getD "") then
           some (match subj with
                 | .variant tag payload => if tag == c then (match pc[1]? with | some sp => matchPat m sp payload | none => .ok (some [])) else .ok none
                 | _ => .ok none)
         else none
       | none => none)
      <|>
      -- a QUALIFIED built-in Option/Result ctor pattern `((. Option Some) p)` / `(. Option None)` /
      -- `((. Result Ok) p)` / `((. Result Err) p)` — the bare `(Some p)`/… forms are handled by the
      -- name-head branch below; the qualified forms match the SAME built-in Value.some/none/ok/err.
      (match qualHead? m pc with
       | some (q, c) =>
         if q == "Option".toUTF8 && c == "Some".toUTF8 then
           some (match subj, pc[1]? with | .some p, some sp => matchPat m sp p | .some _, none => .ok (some []) | _, _ => .ok none)
         else if q == "Option".toUTF8 && c == "None".toUTF8 then
           some (match subj with | .none => .ok (some []) | _ => .ok none)
         else if q == "Result".toUTF8 && c == "Ok".toUTF8 then
           some (match subj, pc[1]? with | .ok p, some sp => matchPat m sp p | .ok _, none => .ok (some []) | _, _ => .ok none)
         else if q == "Result".toUTF8 && c == "Err".toUTF8 then
           some (match subj, pc[1]? with | .err p, some sp => matchPat m sp p | .err _, none => .ok (some []) | _, _ => .ok none)
         else none
       | none => none)
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
         | .tuple es =>
           let sps := pc.extract 1 pc.size
           -- a TRAILING `(.. rest)` binds `rest` to the RESIDUAL sub-tuple (a fresh tuple of the elements
           -- past the leading positional patterns; possibly empty at exact arity). A tuple has static
           -- arity, so this is irrefutable once the leading count is available.
           match restBinderOf? m sps with
           | some (leadCount, binderId) =>
             let leadPats := sps.extract 0 leadCount
             if es.size < leadPats.size then .ok none
             else
               let leadPairs := (leadPats.zip (es.extract 0 leadPats.size)).toList
               let restTuple := Value.tuple (es.extract leadPats.size es.size)
               matchSeq m (leadPairs ++ [(binderId, restTuple)])
           | none =>
             if sps.size != es.size then .ok none else matchSeq m (sps.zip es).toList
         | _ => .ok none)
      else if ph == "record".toUTF8 then
        (match subj with
         | .record fields =>
           let fps := pc.extract 1 pc.size
           -- a TRAILING `(.. rest)` binds `rest` to a record of the RESIDUAL (unnamed) fields — those not
           -- captured by a leading `(= key p)` field pattern (#6750). Leading patterns must all be named.
           match restBinderOf? m fps with
           | some (leadCount, binderId) =>
             let leadPats := (fps.extract 0 leadCount).toList
             let leadKeys := leadPats.filterMap (fun fp => (recordField? m fp).map (·.1))
             if leadKeys.length != leadPats.length then .error (.unsupported "eval: record rest-pattern leading field is not (= key p)")
             else
               match matchRecordPats m leadPats fields with
               | .ok (some e1) =>
                 let restFields := fields.filter (fun kv => !(leadKeys.any (fun k => k == kv.1)))
                 (match matchPat m binderId (Value.record restFields) with
                  | .ok (some e2) => .ok (some (e1 ++ e2))
                  | r => r)
               | r => r
           | none => matchRecordPats m fps.toList fields
         | _ => .ok none)
      else if ph == "map".toUTF8 then
        -- a map pattern `(map (k p)…)` — each entry's key literal must be present with a matching value —
        -- optionally ending in `.. rest`, which binds the REMAINING entries (those not named by a leading
        -- key) as a map. Without `..`: exact key-count decides; a FEWER-key (subset) pattern with no rest
        -- is a skip (unmodeled — never a wrong arm selection).
        (match subj with
         | .map entries =>
           let eps := pc.extract 1 pc.size
           match eps.findIdx? (fun e => nameOf? m e == some "..".toUTF8) with
           | some kk =>
             match eps[kk+1]? with
             | some restBinder =>
               let leading := (eps.extract 0 kk).toList
               -- the leading entries' KEY literals (to compute the rest = entries minus these keys)
               let leadKeys := leading.filterMap (fun ep => match m.nodes[ep]? with
                 | some (Node.list ec) => (ec[0]?).bind (fun kn => (m.nodes[kn]?).bind (fun n =>
                     match n with | .atom lid => (m.leaves[lid]?).bind Value.ofLeaf | _ => none))
                 | _ => none)
               if leadKeys.length != leading.length then .error (.unsupported "eval: map rest-pattern key not a scalar literal")
               else
                 match matchMapPats m leading entries with
                 | .ok (some e1) =>
                   let restEntries := entries.filter (fun e => !(leadKeys.any (fun k => k == e.1)))
                   (match matchPat m restBinder (Value.map restEntries) with
                    | .ok (some e2) => .ok (some (e1 ++ e2))
                    | r => r)
                 | r => r
             | none => .error (.unsupported "eval: malformed map rest pattern (no binder after ..)")
           | none =>
             if eps.size > entries.size then .ok none
             else if eps.size < entries.size then .error (.unsupported "eval: map subset-pattern not modeled")
             else matchMapPats m eps.toList entries
         | _ => .ok none)
      else if ph == "list".toUTF8 then
        -- a list pattern: fixed `(list p0 … pn)` (arity-checked positional) or a rest pattern binding
        -- `rest` to the REMAINING elements as a list. The rest marker is either a BARE `..` + next binder
        -- (`(list p0 … .. rest)`) or a GROUPED `(.. rest)` node (`#list(p0 … (.. rest))`) — both via
        -- `restBinderOf?`, so a literal/ctor-element list pattern with a trailing grouped rest matches.
        (match subj with
         | .list es =>
           let sps := pc.extract 1 pc.size
           match restBinderOf? m sps with
           | some (leadCount, restBinder) =>
             if es.size < leadCount then .ok none
             else
               let leading := (sps.extract 0 leadCount).toList.zip (es.extract 0 leadCount).toList
               matchSeq m (leading ++ [(restBinder, Value.list (es.extract leadCount es.size))])
           | none =>
             if sps.size != es.size then .ok none else matchSeq m (sps.zip es).toList
         | _ => .ok none)
      else if ph == "quasiquote".toUTF8 then
        -- a QUOTE-pattern `(quasiquote <template>)`: structurally match the `Ast` scrutinee against the
        -- template, `(unquote X)` positions binding. (A plain `(quote L)` literal pattern is the leaf
        -- case of the same idea; the corpus uses the quasiquote form for binders.)
        (match pc[1]? with
         | some templateId => matchQuasiPat m templateId subj
         | none => .error (.unsupported "eval: malformed quasiquote pattern"))
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

/-- Match a QUASIQUOTE-pattern template `templateId` against an `Ast` VALUE `subj` (metaprogramming
quote-patterns, e.g. `((quasiquote (+ (unquote a) (unquote b))) …)`): a template `(unquote X)` position
BINDS (`matchPat X` against the Ast subvalue); a template list matches an `Ast.List` value structurally
+ positionally (FIXED arity — a size mismatch is no-match, so a wrong-arity scrutinee falls to the next
arm); a template leaf matches the corresponding `Ast` leaf variant BY VALUE (via `quoteReflect` equality).
This is the pattern dual of quasiquote construction, and equivalent to the `((. Ast …) …)` ctor pattern. -/
partial def matchQuasiPat (m : Module) (templateId : Nat) (subj : Value) : Except Outcome (Option Env) :=
  match m.nodes[templateId]? with
  | some (Node.list tc) =>
    if m.headName? (Node.list tc) == some "unquote".toUTF8 then
      match tc[1]? with
      | some binderId => matchPat m binderId subj
      | none => .error (.unsupported "eval: malformed unquote in quote-pattern")
    else
      -- a compound template → the subj must be an `Ast.List`. Children match positionally; a TRAILING
      -- `(unquote-splicing X)` binds X to the REMAINING Ast children as a list (like a list `.. rest`).
      match subj with
      | .variant tag (.list es) =>
        if tag != "List".toUTF8 then .ok none
        else
          match tc.findIdx? (fun t => (m.nodes[t]?).bind (fun n => m.headName? n) == some "unquote-splicing".toUTF8) with
          | some k =>
            if k != tc.size - 1 then .error (.unsupported "eval: non-final unquote-splicing in quote-pattern not modeled")
            else if es.size < k then .ok none
            else
              match m.nodes[tc[k]!]? with
              | some (Node.list sc) =>
                match sc[1]? with
                | some binderId =>
                  (match matchQuasiSeq m ((tc.extract 0 k).toList.zip (es.extract 0 k).toList) with
                   | .ok (some e1) =>
                     (match matchPat m binderId (Value.list (es.extract k es.size)) with
                      | .ok (some e2) => .ok (some (e1 ++ e2))
                      | r => r)
                   | r => r)
                | none => .error (.unsupported "eval: malformed unquote-splicing in quote-pattern")
              | _ => .error (.unsupported "eval: malformed unquote-splicing in quote-pattern")
          | none =>
            if tc.size == es.size then matchQuasiSeq m (tc.zip es).toList else .ok none
      | _ => .ok none
  | some (Node.atom _) =>
    -- a literal template leaf: subj must EQUAL the leaf's reflected `Ast` variant (structural)
    match quoteReflect m defaultFuel templateId with
    | .value av => (match observeDeep subj with
                    | .value sv => if sv == av then .ok (some []) else .ok none
                    | other => .error other)
    | other => .error other
  | none => .error (.unsupported "eval: quote-pattern template node out of range")

/-- Positional AND of quasiquote-pattern sub-matches (a template child vs the corresponding Ast child). -/
partial def matchQuasiSeq (m : Module) (pairs : List (Nat × Value)) : Except Outcome (Option Env) :=
  match pairs with
  | [] => .ok (some [])
  | (tid, v) :: rest =>
    match matchQuasiPat m tid v with
    | .error o => .error o
    | .ok none => .ok none
    | .ok (some e1) => match matchQuasiSeq m rest with
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

/-- Match map entry-patterns `(k p)…` against a map's entries: each pattern entry's KEY (a scalar
literal) must be a key in the map, its value matched by the entry's sub-pattern. -/
partial def matchMapPats (m : Module) (entryPats : List Nat) (entries : Array (Value × Value)) : Except Outcome (Option Env) :=
  match entryPats with
  | [] => .ok (some [])
  | ep :: rest =>
    match m.nodes[ep]? with
    | some (Node.list ec) =>
      match ec[0]?, ec[1]? with
      | some kNode, some pNode =>
        match m.nodes[kNode]? with
        | some (Node.atom lid) =>
          match (m.leaves[lid]?).bind Value.ofLeaf with
          | some kv =>
            match (entries.find? (fun e => e.1 == kv)).map (·.2) with
            | some vv =>
              (match matchPat m pNode vv with
               | .error o => .error o
               | .ok none => .ok none
               | .ok (some e1) => match matchMapPats m rest entries with
                                  | .error o => .error o
                                  | .ok none => .ok none
                                  | .ok (some e2) => .ok (some (e1 ++ e2)))
            | none => .ok none   -- key absent from the map → no match
          | none => .error (.unsupported "eval: map-pattern key is not a scalar literal")
        | _ => .error (.unsupported "eval: map-pattern key is not a literal")
      | _, _ => .error (.unsupported "eval: malformed map-pattern entry")
    | _ => .error (.unsupported "eval: malformed map-pattern entry")

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
      -- a GUARDED arm `(guard <pat> <cond>)`: match `<pat>`, and if it binds, evaluate `<cond>` in those
      -- bindings — TRUE takes the arm, FALSE means the arm does NOT match so we fall through to the next
      -- (guard-first ordering: a guarded arm and a later unguarded arm for the same variant are exhaustive).
      match (m.nodes[patId]?).bind (fun n => match n with
              | Node.list gc => if m.headName? (Node.list gc) == some "guard".toUTF8 && gc.size == 3
                                then (match gc[1]?, gc[2]? with | some p, some c => some (p, c) | _, _ => none)
                                else none
              | _ => none) with
      | some (innerPat, condId) =>
        (match evalNode m env defaultIntTy fuel scrutId with
         | .value sv0 =>
           (match observeShallow sv0 with
            | .value sv =>
              (match matchPat m innerPat sv with
               | .error o => some o
               | .ok none => none
               | .ok (some ext) =>
                 (match evalNode m (ext ++ env) defaultIntTy fuel condId with
                  | .value (.bool true) => some (evalNode m (ext ++ env) ty fuel bodyId)
                  | .value (.bool false) => none                    -- guard failed → try the next arm
                  | other => some other))                           -- non-bool / trap / unsupported cond → decide with it
            | other => some other)
         | other => some other)
      | none =>
      match m.nodes[patId]? with
      | some (Node.atom lid) =>
        match m.leaves[lid]? with
        | some (Leaf.name b) =>
          if b == "_".toUTF8 then some (evalNode m env ty fuel bodyId)          -- wildcard: no force
          else some (evalNode m ((b, (Thunk.mk (fun _ => evalNode m env defaultIntTy fuel scrutId)), Option.none) :: env)
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

/-- Does the module carry a `(pragma default-fraction …)` directive? Such a pragma changes the DEFAULT
type of bare numeric literals in scope (e.g. `Rational`, so a bare `0.5` grounds to `1/2` and `(/ 1 2)`
is EXACT rational division `1/2`, not the Int64 truncation `0`). The oracle does not thread a scope-level
default-literal-type, so a program carrying it is a coverage gap → SKIP (rather than grade its literals at
the wrong Int64/Float64 default and emit a false mismatch). Rational grounding via an explicit
`(: … Rational)` annotation IS modeled (evalAscribe); only the implicit pragma default is unmodeled. -/
def containsDefaultFractionPragma? (m : Module) : Bool :=
  m.nodes.any (fun node =>
    match node with
    | Node.list cs =>
      headName? m node == some "pragma".toUTF8 &&
      (cs[1]?).bind (nameOf? m) == some "default-fraction".toUTF8
    | _ => false)

end Eval

/-- STAGE 2 — run a trial against the program: bind the call arguments to `main`'s parameters (with
each param's declared integer type, so narrow-typed params trap at their width) and evaluate its body.
For the no-argument trial this is a nullary `main` with an empty env. -/
def executeExport (m : Ast.Module) (exportName : ByteArray) (args : Array Value) : Outcome :=
  if Eval.containsDefaultFractionPragma? m then
    .unsupported "execute: (pragma default-fraction …) literal defaulting not modeled"
  else match Eval.namedParamsBody? m exportName with
  | some (specs, bodyId) =>
    if specs.size != args.size then
      .unsupported s!"execute: arity mismatch ({specs.size} params, {args.size} args)"
    else
      -- bind each already-evaluated arg value under its parameter name + declared type
      let bindings := (specs.zip args).filterMap (fun (specId, v) =>
        (Eval.paramSpec? m specId).map (fun (nm, ty) =>
          (nm, (Thunk.mk (fun _ => Outcome.value v)), ty)))
      if bindings.size != specs.size then
        .unsupported "execute: a parameter spec is malformed"
      else
        -- the result flows to the program output → observe it DEEPLY, so a deferred poison (a trapping
        -- element of a returned compound) surfaces its trap at the output boundary.
        match Eval.evalNode m bindings.toList Eval.defaultIntTy Eval.defaultFuel bodyId with
        | .value v => Eval.observeDeep v
        -- the export IS the fallible boundary when a top-level `?` short-circuits its body → return Err/None.
        | .errReturn ev => Eval.observeDeep ev
        | other => other
  | none => .unsupported "execute: program has no (def (<export> …) BODY) for the called export"

/-- STAGE 2 — run a trial against `main` (the default export). Kept for the common single-`main` program. -/
def execute (m : Ast.Module) (args : Array Value) : Outcome := executeExport m "main".toUTF8 args

/-- STAGE 1 — const-evaluate a closed program to its minimal form (grades a bare `(input E)` case).
Equal to `execute` with no arguments (stage parity holds by construction). -/
def reduce (m : Ast.Module) : Outcome := execute m #[]

end Oracle
