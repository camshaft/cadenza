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
  | unit
  -- compound values (their canonical forms, per `cdz-run`'s render): Option, Result, tuple, list
  | some (v : Value)
  | none
  | ok (v : Value)
  | err (v : Value)
  | tuple (elems : Array Value)
  | list (elems : Array Value)
  | record (fields : Array (ByteArray × Value))  -- fields sorted by key (canonical, order-insensitive)
  | variant (tag : ByteArray) (payload : Value)  -- a prelude/user sum value `(Ctor payload)` (nullary payload = unit)
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
  | .name b => if b == unitName then Option.some .unit else Option.none
  | _ => Option.none

/-- A SCALAR value as its standalone canonical value-AST module (root atom → the value's leaf). A
compound value is not leaf-encoded here (the scalar round-trip gate never encodes one); it degenerates
to an empty module. -/
def toModule (v : Value) : Module :=
  match v.toLeaf? with
  | Option.some l => { leaves := #[l], nodes := #[Node.atom 0], root := 0 }
  | Option.none => { leaves := #[], nodes := #[], root := 0 }

/-- Interpret a module whose root is an atom → leaf as a scalar value. -/
def ofModule? (m : Module) : Option Value :=
  match m.nodes[m.root]? with
  | Option.some (Node.atom lid) =>
    match m.leaves[lid]? with
    | Option.some l => ofLeaf l
    | Option.none => Option.none
  | _ => Option.none

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
