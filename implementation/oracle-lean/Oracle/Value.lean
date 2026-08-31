/-
The oracle's scalar VALUE domain and its canonical value-AST form.

The oracle computes over a semantic value domain (`Value`) and serializes a value to its **canonical
value-AST** — a standalone binary-AST `Module` (`Oracle.Ast`) — for comparison. Everything crosses as
binary AST: comparing the oracle's produced value to an expected value is canonical-AST byte-equality
(`Value.encode`), never s-expr text (operator directive: all binary AST, no s-expr parse/stringify in
Lean). This is L0.3's scalar layer; `reduce`/`execute` (L1.1) produce `Value`s, and later increments
add compound values (option/result/record/list/sum).

A scalar value's canonical AST is a one-node module: the root is an `Atom` referencing the value's
leaf (an int / bool / string / char leaf), or the `unit` name for the unit value. Integer values
canonicalize to DECIMAL radix with a minimal magnitude, so `Value.encode` is one canonical byte form
per value (a hex-spelled corpus literal and its decimal value encode identically).
-/
import Oracle.Ast

namespace Oracle

open Oracle.Ast

/-- A DEFERRED non-value outcome, carried by a `poison` value. A compound ELEMENT that does not
evaluate to a value (a trap / divergence / unmodeled construct) is stored as a `poison` rather than
propagated eagerly — so an element that is never OBSERVED (never projected, never flowed to the result)
never surfaces its outcome (spec core-semantics.md #A Trap Occurs When Observed). Forcing a poison at an
observation point surfaces the deferred outcome. Mirror of the evaluator's `Outcome` minus `value` (kept
here to keep `Value` self-contained — `Value` is the lower layer `Eval` imports). -/
inductive Deferred where
  | trap (kind : String)
  | diverges
  | unsupported (reason : String)
  deriving Inhabited, BEq

/-- The value domain. Scalars (L0.3) plus compound values (Option/Result/tuple/list/record). A compound
element may be a `poison` (a deferred trap/divergence/unsupported that has not yet been observed). -/
inductive Value where
  | int (n : Int)
  | bool (b : Bool)
  | str (bytes : ByteArray)       -- UTF-8 string content
  | char (bytes : ByteArray)      -- UTF-8 of exactly one scalar
  | bytes (b : ByteArray)         -- a `Bytes` value (raw byte-string literal `b"…"`)
  -- Float values are PASS-THROUGH of the canonical float leaf (we never COMPUTE floats): structural
  -- BEq gives the spec's equality — a single canonical `NaN` form (all NaN equal) and `-0.0` ≠ `0.0`
  -- (the sign bit differs). Floats offer NO total order (compareVals declines) and no arithmetic here.
  | float (neg : Bool) (exp : Int) (sig : ByteArray)   -- a normal float LITERAL (sign + exponent + significand)
  | floatNan                                            -- the one canonical NaN
  | floatInf (neg : Bool)                               -- ±infinity
  -- a COMPUTED f64 (a float arithmetic result): the pass-through `.float` decimal leaf can't be produced by
  -- IEEE compute, so an arithmetic result carries the raw `f64` here. It has no leaf form (never encoded);
  -- `asF64?` reads it directly and `valueEqSpec` compares it f64-bit-exact against any float spelling.
  | f64 (f : Float)
  -- an EXACT Rational value, kept NORMALIZED (den > 0, gcd(|num|,den) = 1) so structural `BEq` is value
  -- equality. Renders as the `num/den` string (a `name` leaf) — see Check.expectedValue?.
  | rational (num den : Int)
  | unit
  -- compound values (their canonical forms, per `cdz-run`'s render): Option, Result, tuple, list
  | some (v : Value)
  | none
  | ok (v : Value)
  | err (v : Value)
  | tuple (elems : Array Value)
  | list (elems : Array Value)
  | record (fields : Array (ByteArray × Value))  -- fields sorted by key (canonical, order-insensitive)
  | set (elems : Array Value)                    -- a Set value; elems kept SORTED + DEDUPED (canonical)
  | map (entries : Array (Value × Value))        -- a Map value; entries kept SORTED BY KEY, deduped (canonical)
  | variant (tag : ByteArray) (payload : Value)  -- a prelude/user sum value `(Ctor payload)` (nullary payload = unit)
  -- a first-class function value: its parameter-spec node ids, its body node id, and its captured
  -- environment (each captured name → its already-forced value, a `poison` if it did not reduce to a
  -- value — so an unused captured binding never surfaces its trap). Closures are never value-equal.
  | closure (params : Array Nat) (body : Nat) (cap : List (ByteArray × Value))
  | poison (d : Deferred)         -- a deferred non-value element outcome, surfaced only when observed
  deriving Inhabited, BEq

namespace Value

/-- Big-endian bytes → `Nat`. -/
def beBytesToNat (b : ByteArray) : Nat :=
  b.toList.foldl (fun acc x => acc * 256 + x.toNat) 0

/-- `Nat` → minimal big-endian bytes (empty for zero, no leading zero byte). -/
partial def natToBeBytes (n : Nat) : ByteArray :=
  if n == 0 then ByteArray.empty
  else (natToBeBytes (n / 256)).push (UInt8.ofNat (n % 256))

/-- The `unit` value's canonical spelling — the `unit` name atom (there is no unit leaf kind). -/
def unitName : ByteArray := "unit".toUTF8

/-- The canonical leaf for a SCALAR value; `none` for a compound value (which has no single leaf).
Integers take decimal radix + minimal magnitude; zero is the empty magnitude with a positive kind. -/
def toLeaf? : Value → Option Leaf
  | .int n => Option.some (.intLit (n < 0) .dec (natToBeBytes n.natAbs))
  | .bool b => Option.some (.boolLit b)
  | .str b => Option.some (.str b)
  | .char b => Option.some (.char b)
  | .bytes b => Option.some (.bytesLit b)
  | .float n e s => Option.some (.float n e s)
  | .floatNan => Option.some .floatNan
  | .floatInf n => Option.some (.floatInf n)
  | .unit => Option.some (.name unitName)
  | _ => Option.none  -- compound values are not leaf-backed

/-- Interpret a leaf as a scalar value, if it is one. A `name "unit"` leaf is the unit value; other
name/symbol/float/bytes/suffixed leaves are not scalar values here. -/
def ofLeaf : Leaf → Option Value
  | .intLit neg _ mag =>
    let n := Int.ofNat (beBytesToNat mag)
    Option.some (.int (if neg then -n else n))
  | .boolLit b => Option.some (.bool b)
  | .str b => Option.some (.str b)
  | .char b => Option.some (.char b)
  | .bytesLit b => Option.some (.bytes b)
  | .float n e s => Option.some (.float n e s)
  | .floatNan => Option.some .floatNan
  | .floatInf n => Option.some (.floatInf n)
  | .name b => if b == unitName then Option.some .unit else Option.none
  -- an `N`-suffixed BigInt literal (`5N`, suffix 0): arbitrary-precision, but the VALUE is just that
  -- integer — the oracle's `.int` is unbounded `Int`, so BigInt arith is exact int arith at `.big` width
  -- (no overflow; see operandTyEnv?).
  | .suffixed 0 (.intBody neg _ mag) =>
    let n := Int.ofNat (beBytesToNat mag)
    Option.some (.int (if neg then -n else n))
  -- an `R`-suffixed Rational literal (`5R`, suffix 1): the exact rational `n/1` (already normalized).
  | .suffixed 1 (.intBody neg _ mag) =>
    let n := Int.ofNat (beBytesToNat mag)
    Option.some (.rational (if neg then -n else n) 1)
  | _ => Option.none

/-- A SCALAR value as its standalone canonical value-AST module (root atom → the value's leaf). A
compound value is not leaf-encoded here (the scalar round-trip gate never encodes one); it degenerates
to an empty module. -/
def toModule (v : Value) : Module :=
  match v with
  -- a RATIONAL is a `(RationalTag <num> <den>)` two-child NODE (rcdzc `KIND_RATIONAL` = 27): the payloadless
  -- rational head leaf + two ordinary int value leaves. Root is the list node over [head, num, den] atoms.
  | .rational num den =>
    { leaves := #[Leaf.rational,
                  Leaf.intLit (num < 0) .dec (natToBeBytes num.natAbs),
                  Leaf.intLit (den < 0) .dec (natToBeBytes den.natAbs)],
      nodes := #[Node.atom 0, Node.atom 1, Node.atom 2, Node.list #[0, 1, 2]],
      root := 3 }
  | _ =>
    match v.toLeaf? with
    | Option.some l => { leaves := #[l], nodes := #[Node.atom 0], root := 0 }
    | Option.none => { leaves := #[], nodes := #[], root := 0 }

/-- Interpret a module whose root is an atom → scalar leaf (or a `(RationalTag num den)` node) as a value. -/
def ofModule? (m : Module) : Option Value :=
  match m.nodes[m.root]? with
  | Option.some (Node.atom lid) =>
    match m.leaves[lid]? with
    | Option.some l => ofLeaf l
    | Option.none => Option.none
  -- RATIONAL node: a list `[head, num, den]` whose head-atom is the `.rational` leaf and whose two other
  -- atoms are int leaves → the exact rational `num/den`.
  | Option.some (Node.list cs) =>
    match cs[0]?, cs[1]?, cs[2]?, cs.size with
    | Option.some hn, Option.some nn, Option.some dn, 3 =>
      match m.nodes[hn]?, m.nodes[nn]?, m.nodes[dn]? with
      | Option.some (Node.atom hl), Option.some (Node.atom nl), Option.some (Node.atom dl) =>
        match m.leaves[hl]?, (m.leaves[nl]?.bind ofLeaf), (m.leaves[dl]?.bind ofLeaf) with
        | Option.some Leaf.rational, Option.some (.int num), Option.some (.int den) =>
          Option.some (.rational num den)
        | _, _, _ => Option.none
      | _, _, _ => Option.none
    | _, _, _, _ => Option.none
  | _ => Option.none

/-- The `f64` a float VALUE denotes — the KEY correction (v-cdz-smith L2 differential, 2026-08-28): a
Cadenza float literal is ROUNDED to `f64` (rcdzc's model), so the oracle must compare floats by their
`f64` value, NOT by the literal's exact decimal (which diverges on extremes: `1.0e-400`→0.0 underflow,
`1.0e308`→the f64-rounded value, …). `(neg, exp, sig)` denotes `±(beBytesToNat sig) × 10^exp`, rounded
via `Float.ofScientific` (correctly-rounded decimal→f64). -/
def asF64? : Value → Option Float
  | .float neg exp sig =>
    let f := Float.ofScientific (beBytesToNat sig) (decide (exp < 0)) exp.natAbs
    Option.some (if neg then -f else f)
  | .floatNan => Option.some (0.0 / 0.0)
  | .floatInf neg => Option.some (if neg then -(1.0 / 0.0) else 1.0 / 0.0)
  | .f64 f => Option.some f
  | _ => Option.none

/-- Canonical `f64` bits for SPEC float equality: all NaN fold to one bit pattern (spec: a single NaN,
all NaN equal), and `-0.0` (bits `0x8000…`) stays DISTINCT from `0.0` (spec: sign-significant zero).
Every other `f64` is keyed by its bits, so equal value ⟺ equal bits. -/
def specFloatEq (a b : Float) : Bool :=
  let canon := fun (f : Float) => if f.isNaN then (0x7ff8000000000000 : UInt64) else f.toBits
  canon a == canon b

/-- Spec value equality: structural EVERYWHERE except at float components, which compare by `f64` value
(`specFloatEq`). Strictly ⊇ structural `BEq` for floats — two structurally-equal floats are `f64`-equal,
so no hold is ever lost; it only recognizes that two float SPELLINGS of the same `f64` (or an extreme
literal and its f64-rounded output) are equal. Used by the checker's computed-vs-expected comparison. -/
partial def valueEqSpec (a b : Value) : Bool :=
  match asF64? a, asF64? b with
  | Option.some fa, Option.some fb => specFloatEq fa fb
  | Option.some _, _ => false
  | _, Option.some _ => false
  | Option.none, Option.none =>
    match a, b with
    | .some x, .some y => valueEqSpec x y
    | .ok x, .ok y => valueEqSpec x y
    | .err x, .err y => valueEqSpec x y
    | .variant t1 p1, .variant t2 p2 => t1 == t2 && valueEqSpec p1 p2
    | .tuple xs, .tuple ys => xs.size == ys.size && (xs.zip ys).all (fun p => valueEqSpec p.1 p.2)
    | .list xs, .list ys => xs.size == ys.size && (xs.zip ys).all (fun p => valueEqSpec p.1 p.2)
    | .set xs, .set ys => xs.size == ys.size && (xs.zip ys).all (fun p => valueEqSpec p.1 p.2)
    | .record f1, .record f2 =>
      f1.size == f2.size && (f1.zip f2).all (fun p => p.1.1 == p.2.1 && valueEqSpec p.1.2 p.2.2)
    | .map m1, .map m2 =>
      m1.size == m2.size && (m1.zip m2).all (fun p => valueEqSpec p.1.1 p.2.1 && valueEqSpec p.1.2 p.2.2)
    | _, _ => a == b

/-- Encode a value to its canonical value-AST bytes (`cdzast\x00\x01`). -/
def encode (v : Value) : ByteArray := Ast.encode v.toModule

/-- Decode canonical value-AST bytes to a scalar value; refuses non-canonical / non-scalar input
(the `Ast.decode` validity rules plus "root is an atom to a scalar leaf"). -/
def decode (bytes : ByteArray) : Except String Value := do
  let m ← Ast.decode bytes
  match ofModule? m with
  | Option.some v => .ok v
  | Option.none => .error "value: not a canonical scalar value-AST (root is not an atom to a scalar leaf)"

end Value

end Oracle
