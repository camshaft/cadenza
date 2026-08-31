/-
`oracle-ast-roundtrip` — the L0.2 + L0.3 gate witness over real corpus-derived module blobs.

For each `program.ast` path given on the command line, it (1) decodes the bytes to a `Module`,
(2) round-trips every SCALAR LEAF through the `Value` codec (L0.3: `Value.decode (Value.encode v)`
must equal `v`), (3) re-encodes the module, and (4) asserts the re-encoded bytes are BYTE-IDENTICAL
to the input. Plus a few decoder-refusal negative cases (L0.2 conformance) and explicit scalar value
round-trips (unit + a negative int + the empty string). Any failure fails the run (non-zero exit).

These are codec-law checks of the oracle's own codecs — not a re-test of corpus semantics
(PRINCIPLES.md §2): every `program.ast` is a canonical `codec::encode` output, so decode∘encode is
the identity, and a scalar value's binary-AST form must survive encode∘decode. Everything is binary
AST; no s-expr is parsed.
-/
import Oracle

open Oracle Oracle.Ast

/-- A scalar leaf, interpreted as a `Value`, must survive `Value.encode`→`Value.decode` unchanged
(canonical value-AST byte codec, L0.3). Non-scalar leaves (name/symbol/float/bytes/suffixed) are not
value leaves here and are skipped. -/
def valueLeafOk (l : Leaf) : Bool :=
  match Value.ofLeaf l with
  | none => true
  | some v =>
    match Value.decode (Value.encode v) with
    | .ok v' => v == v'
    | .error _ => false

/-- A short label of a leaf's kind, for a failure message. -/
def reprLeafKind : Leaf → String
  | .intLit .. => "int" | .float .. => "float" | .str _ => "str" | .boolLit _ => "bool"
  | .name _ => "name" | .bytesLit _ => "bytes" | .badEscape _ => "bad-escape" | .char _ => "char"
  | .badChar _ => "bad-char" | .sym _ => "sym" | .suffixed .. => "suffixed"
  | .floatNan => "nan" | .floatInf _ => "inf"
  | .listCtor => "list-ctor" | .tupleCtor => "tuple-ctor" | .recordCtor => "record-ctor"
  | .mapCtor => "map-ctor" | .setCtor => "set-ctor" | .fieldPair => "field-pair" | .member => "member"
  | .rational => "rational"

/-- Hex of a short byte slice, for a legible mismatch report. -/
def hexPrefix (b : ByteArray) (n : Nat) : String :=
  let m := min n b.size
  String.intercalate " " ((List.range m).map (fun i =>
    let v := (b[i]!).toNat
    let hi := v / 16; let lo := v % 16
    let d := fun x => if x < 10 then Char.ofNat (48 + x) else Char.ofNat (87 + x)
    s!"{d hi}{d lo}"))

/-- First index where two byte arrays differ, if any. -/
def firstDiff (a b : ByteArray) : Option Nat :=
  let n := max a.size b.size
  (List.range n).find? (fun i => a[i]? != b[i]?)

/-- The realized-coverage category of a reduce `Outcome` (for the coverage report). -/
def outcomeCategory : Oracle.Outcome → String
  | .value _ => "value" | .trap _ => "trap" | .diverges => "diverges" | .unsupported _ => "unsupported"
  -- a `?` short-circuit never reaches the top level (the fn boundary converts it) → bucket as unsupported
  | .errReturn _ => "unsupported"

/-- Check one corpus module: AST byte round-trip (L0.2), scalar-value round-trip over its leaves
(L0.3), and reduce SOUNDNESS (L1.1) — reduce is deterministic and stage-parity holds
(`reduce m == execute m #[]`). Returns the reduce category on success (for the coverage histogram),
or `none` on any failure. Totality is checked implicitly: a non-terminating/panicking reduce would
crash the run rather than return here. -/
def checkModule (path : String) : IO (Option String) := do
  let bytes ← IO.FS.readBinFile path
  match decode bytes with
  | .error e =>
    IO.eprintln s!"FAIL {path}: decode error: {e}"; return none
  | .ok m =>
    -- L0.3: scalar value round-trip over every leaf.
    if let some bad := m.leaves.toList.find? (fun l => !valueLeafOk l) then
      IO.eprintln s!"FAIL {path}: scalar value round-trip failed for a leaf ({reprLeafKind bad})"
      return none
    -- L0.2: AST byte-identical round-trip.
    let re := encode m
    if re != bytes then
      IO.eprintln s!"FAIL {path}: re-encode not byte-identical (in={bytes.size}B out={re.size}B)"
      match firstDiff bytes re with
      | some i => IO.eprintln s!"  first diff at byte {i}: in {hexPrefix (bytes.extract (i - min i 4) (min bytes.size (i + 8))) 12} / out {hexPrefix (re.extract (i - min i 4) (min re.size (i + 8))) 12}"
      | none => pure ()
      return none
    -- L1.1: reduce soundness — determinism + stage parity (reduce == execute with no args).
    let r1 := Oracle.reduce m
    let r2 := Oracle.reduce m
    if r1 != r2 then
      IO.eprintln s!"FAIL {path}: reduce is non-deterministic"; return none
    if r1 != Oracle.execute m #[] then
      IO.eprintln s!"FAIL {path}: stage parity broken (reduce ≠ execute with no args)"; return none
    return some (outcomeCategory r1)

/-- Resolve the fixture paths from argv: either `--manifest FILE` (newline-separated paths, the
robust form for thousands of blobs) or the paths given directly. -/
def resolvePaths (args : List String) : IO (List String) := do
  match args with
  | ["--manifest", file] =>
    let text ← IO.FS.readFile file
    -- `String.trim` is deprecated in favor of `String.trimAscii`, but the latter returns a
    -- `String.Slice` (type-incompatible with the `≠ ""` filter here); `trim` is correct for
    -- stripping the manifest's line endings.
    return (text.splitOn "\n").map String.trim |>.filter (· ≠ "")
  | _ => return args

/-- Build a `ByteArray` from a list of byte values. -/
def bytesOf (l : List Nat) : ByteArray := ByteArray.mk ((l.map (fun n => UInt8.ofNat n)).toArray)

/-- The 8-byte `cdzast\x00\x01` header. -/
def hdr : List Nat := [0x63, 0x64, 0x7a, 0x61, 0x73, 0x74, 0x00, 0x01]

/-- Assert `decode` REFUSES a crafted-malformed module (per spec/contracts/ast-binary-format.md). -/
def expectReject (label : String) (body : List Nat) : IO Bool := do
  match decode (bytesOf (hdr ++ body)) with
  | .error _ => return true
  | .ok _ =>
    IO.eprintln s!"NEG FAIL {label}: expected decode to be REFUSED, but it was accepted"
    return false

/-- The decoder's spec-mandated refusals (no corpus case exercises malformed input). -/
def negativeChecks : IO Bool := do
  let mut ok := true
  -- non-minimal varu64 (leaf count 0 encoded over-long as 80 00)
  ok := (← expectReject "over-long varu64" [0x80, 0x00]) && ok
  -- trailing byte after a valid tiny module (leaf name "a", one atom, root 0) + 0xFF
  ok := (← expectReject "trailing bytes"
    [0x01, 0x0a, 0x01, 0x61, 0x01, 0x00, 0x00, 0x00, 0xff]) && ok
  -- not a tree: a list [0,0] reaches the same atom node twice
  ok := (← expectReject "not a tree (shared node)"
    [0x01, 0x0a, 0x01, 0x61, 0x02, 0x00, 0x00, 0x01, 0x02, 0x00, 0x00, 0x01]) && ok
  -- char leaf encoding two scalars ("ab") — must be exactly one
  ok := (← expectReject "char leaf with two scalars"
    [0x01, 0x0d, 0x02, 0x61, 0x62, 0x01, 0x00, 0x00, 0x00]) && ok
  if ok then IO.println "oracle-ast-roundtrip: negative checks ok (4 malformed inputs refused)"
  return ok

/-- Scalar Value round-trips not guaranteed to appear as corpus program leaves: the unit value (no
leaf kind — the `unit` name atom), a negative int, and the empty string. -/
def valueScalarChecks : IO Bool := do
  let cases : List (String × Value) :=
    [("unit", .unit), ("neg int", .int (-42)), ("zero", .int 0), ("empty string", .str "".toUTF8)]
  let mut ok := true
  for (label, v) in cases do
    match Value.decode (Value.encode v) with
    | .ok v' => if v != v' then do IO.eprintln s!"VALUE FAIL {label}: round-trip changed the value"; ok := false
    | .error e => do IO.eprintln s!"VALUE FAIL {label}: {e}"; ok := false
  if ok then IO.println "oracle-ast-roundtrip: scalar value checks ok (unit + explicit scalars round-trip)"
  return ok

def main (args : List String) : IO UInt32 := do
  if args.isEmpty then
    IO.eprintln "oracle-ast-roundtrip: usage: oracle-ast-roundtrip (--manifest FILE | <program.ast>...)"
    return 2
  let negOk ← negativeChecks
  let valOk ← valueScalarChecks
  let paths ← resolvePaths args
  if paths.isEmpty then
    IO.eprintln "oracle-ast-roundtrip: no fixture paths given"
    return 2
  let mut ok := 0
  let mut fail := 0
  let mut nValue := 0
  let mut nTrap := 0
  let mut nDiverges := 0
  let mut nUnsupported := 0
  for path in paths do
    match ← checkModule path with
    | none => fail := fail + 1
    | some cat =>
      ok := ok + 1
      match cat with
      | "value" => nValue := nValue + 1
      | "trap" => nTrap := nTrap + 1
      | "diverges" => nDiverges := nDiverges + 1
      | _ => nUnsupported := nUnsupported + 1
  IO.println s!"oracle-ast-roundtrip: {ok} ok, {fail} failed (of {paths.length}) — AST + value round-trip + reduce soundness"
  IO.println s!"oracle reduce coverage: {nValue} value, {nTrap} trap, {nDiverges} diverges, {nUnsupported} unsupported (of {ok})"
  return (if fail == 0 && negOk && valOk then 0 else 1)
