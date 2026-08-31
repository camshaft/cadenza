/-
The Cadenza binary AST (`cdzast\x00\x01`) decoded into Lean, plus the mirror encoder.

This is the oracle's INPUT: a module is a leaf pool + a node arena (`Node = Atom leafId | List
children`) + a root index, exactly the frozen `spec/contracts/ast-encoding.md` artifact. A construct
`(head child…)` is a `List` whose first child is an `Atom` referencing a `Name` leaf — there is no
separate symbol-prelude / namespace / version section in the concrete format (the leaf pool plays
that role).

Scope note (clean-room, PRINCIPLES.md §1): the byte codec is shared TRANSPORT — the oracle must
byte-match the frozen format to read the harness's input at all; language bugs do not live here. This
module is validated purely by BYTE-IDENTICAL ROUND-TRIP on real corpus-derived `program.ast` blobs
(`OracleAstTest`), a codec-law check of the decoder itself — not a re-test of any corpus semantics.

The representation is byte-faithful (every field that affects the bytes is preserved) AND
semantically usable (ints carry sign+radix+magnitude, floats sign+exponent+significand, text its
bytes), so the L1.1 evaluator can consume it directly. Leaf order and node order are preserved as
decoded; a canonically-encoded module (which every `program.ast` is) re-encodes byte-identically
without re-running canonicalization.
-/
import Oracle.Leb

namespace Oracle.Ast

open Oracle.Leb

/-- Integer-literal display radix — folded into the leaf kind byte, value-irrelevant but
byte-relevant, so preserved. -/
inductive Radix where
  | dec | hex | bin
  deriving DecidableEq, Inhabited

/-- A type-suffixed numeric literal body (`100N`, `0.5R`). -/
inductive SuffBody where
  | intBody (negative : Bool) (radix : Radix) (mag : ByteArray)
  | floatBody (negative : Bool) (exponent : Int) (sig : ByteArray)
  deriving Inhabited

/-- One deduplicated primitive leaf. Text/byte payloads are kept as raw `ByteArray` (UTF-8 validation
is deferred to where the semantics need it — irrelevant to the byte round-trip). -/
inductive Leaf where
  | intLit (negative : Bool) (radix : Radix) (mag : ByteArray)  -- kinds 0..5
  | float (negative : Bool) (exponent : Int) (sig : ByteArray)  -- kind 6
  | str (bytes : ByteArray)          -- 7
  | boolLit (value : Bool)           -- 8 / 9
  | name (bytes : ByteArray)         -- 10
  | bytesLit (bytes : ByteArray)     -- 11
  | badEscape (bytes : ByteArray)    -- 12
  | char (bytes : ByteArray)         -- 13
  | badChar (bytes : ByteArray)      -- 14
  | sym (bytes : ByteArray)          -- 15
  | suffixed (suffix : UInt8) (body : SuffBody)  -- 16 (suffix 0 = N bigint, 1 = R rational)
  | floatNan                         -- 17
  | floatInf (negative : Bool)       -- 18 / 19
  -- M2 native-compound-data ctor-leaf HEADS (payloadless, single kind byte — like bool/floatNan):
  -- a compound `(tuple e…)` / `(record …)` / `(list …)` / `(map …)` / `(Set.of …)` now leads with the
  -- matching ctor leaf instead of a `name "tuple"` head; `fieldPair` is the `=` record/map-entry head
  -- and `member` the `.` member-access head. (rcdzc codec `KIND_LIST_CTOR`..`KIND_MEMBER` = 20..26.)
  | listCtor                         -- 20
  | tupleCtor                        -- 21
  | recordCtor                       -- 22
  | mapCtor                          -- 23
  | setCtor                          -- 24
  | fieldPair                        -- 25
  | member                           -- 26
  -- The native RATIONAL head (rcdzc codec `KIND_RATIONAL` = 27): the payloadless head of a
  -- `(RationalTag <num> <den>)` two-child node (children = ordinary int value leaves) — a distinct data
  -- type recognized by kind (operator seq-204/207), payloadless like `fieldPair`/`member`.
  | rational                         -- 27
  deriving Inhabited

/-- A structure-arena node. -/
inductive Node where
  | atom (leafId : Nat)
  | list (children : Array Nat)
  deriving Inhabited

/-- A decoded module: the leaf pool, the node arena, and the root node index. -/
structure Module where
  leaves : Array Leaf
  nodes : Array Node
  root : Nat
  deriving Inhabited

/-! ### Constants (from the concrete format; see cadenza-binary-ast-concrete-wire-format) -/

def header : ByteArray := "cdzast\x00\x01".toUTF8

def tagAtom : UInt8 := 0
def tagList : UInt8 := 1

/-! ### Signed exponent as a fixed 8-byte big-endian two's-complement i64 -/

def twoPow64 : Nat := 18446744073709551616  -- 2^64
def twoPow63 : Nat := 9223372036854775808   -- 2^63

/-- Read 8 big-endian bytes as an i64 (two's complement). -/
def readI64BE (c : Cursor) : Except String (Int × Cursor) := do
  let (bs, c) ← c.readBytes 8
  let mut u : Nat := 0
  for i in [0:8] do
    u := (u <<< 8) ||| (bs[i]!).toNat
  let e : Int := if u ≥ twoPow63 then (Int.ofNat u) - (Int.ofNat twoPow64) else Int.ofNat u
  .ok (e, c)

/-- Encode an i64 (two's complement) as 8 big-endian bytes. -/
def writeI64BE (acc : ByteArray) (e : Int) : ByteArray :=
  let u : Nat := if e < 0 then (e + Int.ofNat twoPow64).toNat else e.toNat
  Id.run do
    let mut out := acc
    for i in [0:8] do
      let shift := (7 - i) * 8
      out := out.push (UInt8.ofNat ((u >>> shift) &&& 0xff))
    return out

/-! ### Radix ↔ leaf-kind mapping for integer literals -/

/-- `(negative, radix)` → the int-leaf kind byte (0..5). -/
def intKind (negative : Bool) (radix : Radix) : UInt8 :=
  match negative, radix with
  | false, .dec => 0 | false, .hex => 1 | false, .bin => 2
  | true,  .dec => 3 | true,  .hex => 4 | true,  .bin => 5

/-- The int-leaf kind byte → `(negative, radix)`. -/
def intKindParts (k : UInt8) : Bool × Radix :=
  match k with
  | 0 => (false, .dec) | 1 => (false, .hex) | 2 => (false, .bin)
  | 3 => (true, .dec)  | 4 => (true, .hex)  | _ => (true, .bin)

/-! ### Decoding -/

/-- Read a length-prefixed blob as raw bytes. -/
def readBlob (c : Cursor) : Except String (ByteArray × Cursor) := c.readLenPrefixed

/-- Require `b` to be valid UTF-8 (kinds 7/10/14/15 per the format). Bytes are kept raw; this only
validates, per `spec/contracts/ast-binary-format.md`. -/
def requireUtf8 (what : String) (b : ByteArray) : Except String Unit :=
  match String.fromUTF8? b with
  | some _ => .ok ()
  | none => .error s!"ast: {what} leaf is not valid UTF-8"

/-- Require `b` to be valid UTF-8 encoding exactly ONE Unicode scalar (kinds 12 BadEscape / 13 Char),
so the leaf is injective. -/
def requireOneScalar (what : String) (b : ByteArray) : Except String Unit :=
  match String.fromUTF8? b with
  | some s =>
    if s.length == 1 then .ok ()
    else .error s!"ast: {what} leaf must encode exactly one Unicode scalar (got {s.length})"
  | none => .error s!"ast: {what} leaf is not valid UTF-8"

/-- Decode the `[mag_len][magnitude]` int body given the kind byte. -/
def readIntLeaf (k : UInt8) (c : Cursor) : Except String (Leaf × Cursor) := do
  let (mag, c) ← readBlob c
  let (neg, radix) := intKindParts k
  .ok (.intLit neg radix mag, c)

/-- Decode the `[negative][exp:i64][sig_len][sig]` float body. -/
def readFloatBody (c : Cursor) : Except String ((Bool × Int × ByteArray) × Cursor) := do
  let (nb, c) ← c.readByte
  let (e, c) ← readI64BE c
  let (sig, c) ← readBlob c
  .ok ((nb == 1, e, sig), c)

/-- Decode one leaf: a kind byte then its payload. -/
def readLeaf (c : Cursor) : Except String (Leaf × Cursor) := do
  let (k, c) ← c.readByte
  if k ≤ 5 then
    readIntLeaf k c
  else if k == 6 then
    let ((neg, e, sig), c) ← readFloatBody c
    .ok (.float neg e sig, c)
  else if k == 7 then do let (b, c) ← readBlob c; requireUtf8 "string" b; .ok (.str b, c)
  else if k == 8 then .ok (.boolLit false, c)
  else if k == 9 then .ok (.boolLit true, c)
  else if k == 10 then do let (b, c) ← readBlob c; requireUtf8 "name" b; .ok (.name b, c)
  else if k == 11 then let (b, c) ← readBlob c; .ok (.bytesLit b, c)
  else if k == 12 then do let (b, c) ← readBlob c; requireOneScalar "bad-escape" b; .ok (.badEscape b, c)
  else if k == 13 then do let (b, c) ← readBlob c; requireOneScalar "char" b; .ok (.char b, c)
  else if k == 14 then do let (b, c) ← readBlob c; requireUtf8 "bad-char" b; .ok (.badChar b, c)
  else if k == 15 then do let (b, c) ← readBlob c; requireUtf8 "symbol" b; .ok (.sym b, c)
  else if k == 16 then
    let (suffix, c) ← c.readByte
    let (shape, c) ← c.readByte
    if shape == 0 then
      let (ik, c) ← c.readByte
      let (mag, c) ← readBlob c
      let (neg, radix) := intKindParts ik
      .ok (.suffixed suffix (.intBody neg radix mag), c)
    else if shape == 1 then
      let ((neg, e, sig), c) ← readFloatBody c
      .ok (.suffixed suffix (.floatBody neg e sig), c)
    else
      .error s!"ast: bad suffixed body shape {shape.toNat}"
  else if k == 17 then .ok (.floatNan, c)
  else if k == 18 then .ok (.floatInf false, c)
  else if k == 19 then .ok (.floatInf true, c)
  -- M2 native-compound ctor-leaf heads (payloadless): kinds 20..26.
  else if k == 20 then .ok (.listCtor, c)
  else if k == 21 then .ok (.tupleCtor, c)
  else if k == 22 then .ok (.recordCtor, c)
  else if k == 23 then .ok (.mapCtor, c)
  else if k == 24 then .ok (.setCtor, c)
  else if k == 25 then .ok (.fieldPair, c)
  else if k == 26 then .ok (.member, c)
  else if k == 27 then .ok (.rational, c)
  else
    .error s!"ast: unknown leaf kind {k.toNat}"

/-- Decode one node: a tag byte then its payload. -/
def readNode (c : Cursor) : Except String (Node × Cursor) := do
  let (tag, c) ← c.readByte
  if tag == tagAtom then
    let (id, c) ← c.readUleb
    .ok (.atom id, c)
  else if tag == tagList then
    let (n, c) ← c.readUleb
    let rec go (c : Cursor) (i : Nat) (acc : Array Nat) : Except String (Array Nat × Cursor) := do
      match i with
      | 0 => .ok (acc, c)
      | i + 1 => let (id, c) ← c.readUleb; go c i (acc.push id)
    let (children, c) ← go c n #[]
    .ok (.list children, c)
  else
    .error s!"ast: unknown node tag {tag.toNat} (only Atom=0/List=1 in the canonical plane)"

/-- Read a `uleb`-count-prefixed array of decoded items. -/
def readCountArray (c : Cursor) (f : Cursor → Except String (α × Cursor)) :
    Except String (Array α × Cursor) := do
  let (n, c) ← c.readUleb
  let rec go (c : Cursor) (i : Nat) (acc : Array α) : Except String (Array α × Cursor) := do
    match i with
    | 0 => .ok (acc, c)
    | i + 1 => let (x, c) ← f c; go c i (acc.push x)
  go c n #[]

/-- Referential-integrity check: every atom's leafId is in range, every list child is in range, and
the root is in range. (The tree-ness check is deferred; canonical input is always a tree.) -/
def checkRefs (m : Module) : Except String Unit := do
  let nLeaves := m.leaves.size
  let nNodes := m.nodes.size
  if m.root ≥ nNodes then
    .error s!"ast: root {m.root} out of range (nodes={nNodes})"
  for node in m.nodes do
    match node with
    | .atom id => if id ≥ nLeaves then .error s!"ast: atom leafId {id} out of range (leaves={nLeaves})"
    | .list children =>
      for ch in children do
        if ch ≥ nNodes then .error s!"ast: list child {ch} out of range (nodes={nNodes})"
  .ok ()

/-- Tree-ness check: the structure reachable from the root MUST be a tree — every reachable node is
reached exactly once (a node reached twice, by a shared subtree or a cycle, is refused). Unreachable
nodes are permitted. Assumes referential integrity (checked first), so every index is in range. -/
partial def checkTreeGo (m : Module) (visited : Array Bool) (i : Nat) :
    Except String (Array Bool) :=
  match visited[i]? with
  | some true =>
    .error s!"ast: node {i} reached more than once — not a tree (shared subtree or cycle)"
  | _ =>
    let visited := visited.set! i true
    match m.nodes[i]? with
    | some (Node.list children) => children.foldlM (fun v j => checkTreeGo m v j) visited
    | _ => .ok visited

def checkTree (m : Module) : Except String Unit := do
  let _ ← checkTreeGo m (Array.replicate m.nodes.size false) m.root
  .ok ()

/-- Decode a full module (`cdzast\x00\x01`). Enforces header, referential integrity, tree-ness, and
exact consumption, per `spec/contracts/ast-binary-format.md`. -/
def decode (bytes : ByteArray) : Except String Module := do
  let c := Cursor.ofBytes bytes
  let (hdr, c) ← c.readBytes header.size
  if hdr != header then
    .error "ast: bad header (expected cdzast\\x00\\x01)"
  else
    let (leaves, c) ← readCountArray c readLeaf
    let (nodes, c) ← readCountArray c readNode
    let (root, c) ← c.readUleb
    if !c.atEnd then
      .error s!"ast: {c.remaining} trailing byte(s) after module"
    else
      let m : Module := { leaves, nodes, root }
      checkRefs m
      checkTree m
      .ok m

/-! ### Encoding (the mirror; byte-identical to the canonical serialization of a canonical module) -/

/-- Append the float body `[negative][exp:i64][sig_len][sig]`. -/
def writeFloatBody (acc : ByteArray) (negative : Bool) (exponent : Int) (sig : ByteArray) : ByteArray :=
  let acc := acc.push (if negative then 1 else 0)
  let acc := writeI64BE acc exponent
  (encodeInto acc sig.size) ++ sig

/-- Append one leaf. Mirrors the encoder's `neg = negative && !mag.isEmpty` rule so a zero magnitude
never takes a negative kind. -/
def writeLeaf (acc : ByteArray) (leaf : Leaf) : ByteArray :=
  match leaf with
  | .intLit negative radix mag =>
    let neg := negative && !mag.isEmpty
    let acc := acc.push (intKind neg radix)
    (encodeInto acc mag.size) ++ mag
  | .float negative exponent sig =>
    writeFloatBody (acc.push 6) negative exponent sig
  | .str b => ((acc.push 7).append (encodeInto ByteArray.empty b.size)) ++ b
  | .boolLit false => acc.push 8
  | .boolLit true => acc.push 9
  | .name b => ((acc.push 10).append (encodeInto ByteArray.empty b.size)) ++ b
  | .bytesLit b => ((acc.push 11).append (encodeInto ByteArray.empty b.size)) ++ b
  | .badEscape b => ((acc.push 12).append (encodeInto ByteArray.empty b.size)) ++ b
  | .char b => ((acc.push 13).append (encodeInto ByteArray.empty b.size)) ++ b
  | .badChar b => ((acc.push 14).append (encodeInto ByteArray.empty b.size)) ++ b
  | .sym b => ((acc.push 15).append (encodeInto ByteArray.empty b.size)) ++ b
  | .suffixed suffix body =>
    let acc := (acc.push 16).push suffix
    match body with
    | .intBody negative radix mag =>
      let neg := negative && !mag.isEmpty
      let acc := (acc.push 0).push (intKind neg radix)
      (encodeInto acc mag.size) ++ mag
    | .floatBody negative exponent sig =>
      writeFloatBody (acc.push 1) negative exponent sig
  | .floatNan => acc.push 17
  | .floatInf false => acc.push 18
  | .floatInf true => acc.push 19
  -- M2 native-compound ctor-leaf heads (payloadless): kinds 20..26.
  | .listCtor => acc.push 20
  | .tupleCtor => acc.push 21
  | .recordCtor => acc.push 22
  | .mapCtor => acc.push 23
  | .setCtor => acc.push 24
  | .fieldPair => acc.push 25
  | .member => acc.push 26
  | .rational => acc.push 27

/-- Append one node. -/
def writeNode (acc : ByteArray) (node : Node) : ByteArray :=
  match node with
  | .atom id => encodeInto (acc.push tagAtom) id
  | .list children =>
    children.foldl (fun a ch => encodeInto a ch) (encodeInto (acc.push tagList) children.size)

/-- Encode a module to canonical `cdzast\x00\x01` bytes. -/
def encode (m : Module) : ByteArray := Id.run do
  let mut acc := header
  acc := encodeInto acc m.leaves.size
  for leaf in m.leaves do
    acc := writeLeaf acc leaf
  acc := encodeInto acc m.nodes.size
  for node in m.nodes do
    acc := writeNode acc node
  acc := encodeInto acc m.root
  return acc

/-- Convenience: the head symbol of a `List` node, as raw name bytes, if its first child is an
`Atom` pointing at a `Name` leaf. (Used by later semantic increments; here it documents the
`(head child…)` convention.) -/
def Module.headName? (m : Module) (node : Node) : Option ByteArray :=
  match node with
  | .list children =>
    match children[0]? with
    | some cid =>
      match m.nodes[cid]? with
      | some (Node.atom lid) =>
        match m.leaves[lid]? with
        | some (Leaf.name b) => some b
        -- M2 native-compound ctor-leaf heads decode to the SAME canonical head SYMBOL a pre-M2
        -- `name`-head node carried, so all the name-based head dispatch (tuple/list/record/map/set
        -- construction, `.` member-access, `=` field pair) keeps working unchanged.
        | some Leaf.tupleCtor => some "tuple".toUTF8
        | some Leaf.listCtor => some "list".toUTF8
        | some Leaf.recordCtor => some "record".toUTF8
        | some Leaf.mapCtor => some "map".toUTF8
        | some Leaf.setCtor => some "set".toUTF8
        | some Leaf.member => some ".".toUTF8
        | some Leaf.fieldPair => some "=".toUTF8
        | _ => none
      | _ => none
    | none => none
  | _ => none

end Oracle.Ast
