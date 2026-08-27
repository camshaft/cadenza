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

/-- The scalar value domain (L0.3). Width is a type-level concern handled by the evaluator; a value
carries only its magnitude/sign. Compound values arrive in later increments. -/
inductive Value where
  | int (n : Int)
  | bool (b : Bool)
  | str (bytes : ByteArray)       -- UTF-8 string content
  | char (bytes : ByteArray)      -- UTF-8 of exactly one scalar
  | unit
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

/-- The canonical leaf for a scalar value (total). Integers take decimal radix + minimal magnitude;
zero is the empty magnitude with a positive kind. -/
def toLeaf : Value → Leaf
  | .int n => .intLit (n < 0) .dec (natToBeBytes n.natAbs)
  | .bool b => .boolLit b
  | .str b => .str b
  | .char b => .char b
  | .unit => .name unitName

/-- Interpret a leaf as a scalar value, if it is one. A `name "unit"` leaf is the unit value; other
name/symbol/float/bytes/suffixed leaves are not scalar values here. -/
def ofLeaf : Leaf → Option Value
  | .intLit neg _ mag =>
    let n := Int.ofNat (beBytesToNat mag)
    some (.int (if neg then -n else n))
  | .boolLit b => some (.bool b)
  | .str b => some (.str b)
  | .char b => some (.char b)
  | .name b => if b == unitName then some .unit else none
  | _ => none

/-- A value as its standalone canonical value-AST module: root atom → the value's leaf. -/
def toModule (v : Value) : Module :=
  { leaves := #[v.toLeaf], nodes := #[Node.atom 0], root := 0 }

/-- Interpret a module whose root is an atom → leaf as a scalar value. -/
def ofModule? (m : Module) : Option Value :=
  match m.nodes[m.root]? with
  | some (Node.atom lid) =>
    match m.leaves[lid]? with
    | some l => ofLeaf l
    | none => none
  | _ => none

/-- Encode a value to its canonical value-AST bytes (`cdzast\x00\x01`). -/
def encode (v : Value) : ByteArray := Ast.encode v.toModule

/-- Decode canonical value-AST bytes to a scalar value; refuses non-canonical / non-scalar input
(the `Ast.decode` validity rules plus "root is an atom to a scalar leaf"). -/
def decode (bytes : ByteArray) : Except String Value := do
  let m ← Ast.decode bytes
  match ofModule? m with
  | some v => .ok v
  | none => .error "value: not a canonical scalar value-AST (root is not an atom to a scalar leaf)"

end Value

end Oracle
