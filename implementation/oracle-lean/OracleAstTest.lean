/-
`oracle-ast-roundtrip` — the L0.2 gate witness: the binary-AST codec is byte-identical on real
corpus-derived module blobs.

For each `program.ast` path given on the command line, it (1) decodes the bytes to a `Module`,
(2) re-encodes, and (3) asserts the re-encoded bytes are BYTE-IDENTICAL to the input. Any decode
error or byte mismatch fails the run (non-zero exit). This is a codec-law check of the oracle's own
decoder — not a re-test of any corpus semantics (PRINCIPLES.md §2): every `program.ast` is a
canonical `codec::encode` output, so decode∘encode must be the identity on it.
-/
import Oracle

open Oracle.Ast

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

def roundtripOne (path : String) : IO Bool := do
  let bytes ← IO.FS.readBinFile path
  match decode bytes with
  | .error e =>
    IO.eprintln s!"FAIL {path}: decode error: {e}"
    return false
  | .ok m =>
    let re := encode m
    if re == bytes then
      return true
    else
      IO.eprintln s!"FAIL {path}: re-encode not byte-identical (in={bytes.size}B out={re.size}B)"
      match firstDiff bytes re with
      | some i =>
        IO.eprintln s!"  first diff at byte {i}"
        IO.eprintln s!"  in : {hexPrefix (bytes.extract (i - min i 4) (min bytes.size (i + 8))) 12}"
        IO.eprintln s!"  out: {hexPrefix (re.extract (i - min i 4) (min re.size (i + 8))) 12}"
      | none => pure ()
      return false

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

def main (args : List String) : IO UInt32 := do
  if args.isEmpty then
    IO.eprintln "oracle-ast-roundtrip: usage: oracle-ast-roundtrip (--manifest FILE | <program.ast>...)"
    return 2
  let negOk ← negativeChecks
  let paths ← resolvePaths args
  if paths.isEmpty then
    IO.eprintln "oracle-ast-roundtrip: no fixture paths given"
    return 2
  let mut ok := 0
  let mut fail := 0
  for path in paths do
    if ← roundtripOne path then ok := ok + 1 else fail := fail + 1
  IO.println s!"oracle-ast-roundtrip: {ok} byte-identical, {fail} failed (of {paths.length})"
  return (if fail == 0 && negOk then 0 else 1)
