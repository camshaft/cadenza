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
    return (text.splitOn "\n").map String.trim |>.filter (· ≠ "")
  | _ => return args

def main (args : List String) : IO UInt32 := do
  if args.isEmpty then
    IO.eprintln "oracle-ast-roundtrip: usage: oracle-ast-roundtrip (--manifest FILE | <program.ast>...)"
    return 2
  let paths ← resolvePaths args
  if paths.isEmpty then
    IO.eprintln "oracle-ast-roundtrip: no fixture paths given"
    return 2
  let mut ok := 0
  let mut fail := 0
  for path in paths do
    if ← roundtripOne path then ok := ok + 1 else fail := fail + 1
  IO.println s!"oracle-ast-roundtrip: {ok} byte-identical, {fail} failed (of {paths.length})"
  return (if fail == 0 then 0 else 1)
