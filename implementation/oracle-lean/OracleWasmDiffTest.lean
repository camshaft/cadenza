/-
`oracle-wasm-diff` — the Core↔wasm DIFFERENTIAL conformance runner (v-lean-oracle's half of the
"assert compilation was correct all the way through" pipeline; design: WASM.md, `Oracle.WasmDiff`).

Reads a MANIFEST (one per-case DIRECTORY path per line) — the shape confirmed with v-wasm-oracle. Each
case dir has three files produced by v-wasm-oracle's extractor (a nix derivation, co-designed with v-nix):
  core.wat         — `wasm-tools print` of the unbundled core module (the `coreWat` string)
  result-type.ast  — raw bytes of the emitted `@custom "cdz-result-type"` section (`rtBytes`)
  core.ast         — the Core binary-AST (`coreAst`, = `cdz compile <case> --target cadenza`)
For each case it runs `Oracle.WasmDiff.differential` (Core `reduce coreAst` vs `runWasmWith drive …`) and
tallies agree/diverge/skip; a DIVERGENCE is a real MISCOMPILE finding (printed for triage). Exit nonzero iff
any case diverged.

DRIVER: `Oracle.Wasm.talosDriver` (module `Oracle.Wasm.Talos`) — the talos small-step wasm interpreter,
now on main (`nix/talos-lean-deps` co-land, toolchain 4.32.2). Import-free scalar/arith core modules RUN
end-to-end (Core `reduce` vs talos-interpret); a runtime-importing/heap module returns `.err` →
`.unsupported` → `differential` `.skip` (a sound coverage gap, not a false verdict).
-/
import Oracle
import Oracle.Wasm.Talos

open Oracle Oracle.Wasm Oracle.WasmDiff

/-- Parse a manifest: newline-separated per-case directory paths (blank lines ignored). -/
def readManifest (file : String) : IO (List String) := do
  let text ← IO.FS.readFile file
  return (text.splitOn "\n").map String.trim |>.filter (· ≠ "")

/-- Load one case dir's `(id, coreAst, coreWat, rtBytes)`; `none` (skipped) if `core.ast` fails to decode. -/
def loadCase (dir : String) : IO (Option (String × Ast.Module × String × ByteArray)) := do
  let astBytes ← IO.FS.readBinFile (dir ++ "/core.ast")
  match Ast.decode astBytes with
  | .error e => IO.eprintln s!"SKIP {dir}: core.ast decode error: {e}"; return none
  | .ok m =>
    let wat ← IO.FS.readFile (dir ++ "/core.wat")
    let rtBytes ← IO.FS.readBinFile (dir ++ "/result-type.ast")
    return some (dir, m, wat, rtBytes)

/-- Render a `Value` for a divergence report: shape + scalar values (enough to distinguish e.g. a `tuple`
Core result from a `int` wasm result — the compound-vs-scalar mismatch that triages a resolver leak).
`Outcome`/`Value` have no `Repr`, so this is a purpose-built renderer (no string content for str/char to
avoid noise; the shape is the triage datum). -/
partial def valueStr : Value → String
  | .int n => "int " ++ toString n
  | .bool b => "bool " ++ toString b
  | .str _ => "str"
  | .char _ => "char"
  | .bytes _ => "bytes"
  | .float _ _ _ => "float"
  | .floatNan => "NaN"
  | .floatInf _ => "inf"
  | .f64 f => "f64 " ++ toString f
  | .rational n d => "rational " ++ toString n ++ "/" ++ toString d
  | .unit => "unit"
  | .some v => "Some(" ++ valueStr v ++ ")"
  | .none => "None"
  | .ok v => "Ok(" ++ valueStr v ++ ")"
  | .err v => "Err(" ++ valueStr v ++ ")"
  | .tuple es => "tuple[" ++ String.intercalate "," (es.toList.map valueStr) ++ "]"
  | .list es => "list[" ++ String.intercalate "," (es.toList.map valueStr) ++ "]"
  | .record fs => "record[" ++ String.intercalate "," (fs.toList.map (fun f => valueStr f.2)) ++ "]"
  | .set es => "set[" ++ String.intercalate "," (es.toList.map valueStr) ++ "]"
  | .map es => "map[" ++ String.intercalate "," (es.toList.map (fun kv => valueStr kv.1 ++ "->" ++ valueStr kv.2)) ++ "]"
  | .variant _ _ => "variant"
  | .closure _ _ _ => "closure"
  | .poison _ => "poison"

/-- Render an `Outcome` for a divergence report. -/
def outcomeStr : Outcome → String
  | .value v => "value " ++ valueStr v
  | .trap k => "trap " ++ k
  | .diverges => "diverges"
  | .unsupported r => "unsupported " ++ r
  | .errReturn v => "errReturn " ++ valueStr v

/-- Tally skip REASONS by frequency (descending) — surfaces where the runnable fraction is lost, and (with
v-wasm-oracle's head-tagged reason `… (head=<name>)`) which unmodeled result-type heads dominate. -/
def tallyReasons (rs : List String) : List (String × Nat) :=
  let counts := rs.foldl (fun (acc : List (String × Nat)) r =>
    match acc.find? (fun p => p.1 == r) with
    | some _ => acc.map (fun p => if p.1 == r then (p.1, p.2 + 1) else p)
    | none => acc ++ [(r, 1)]) []
  (counts.toArray.qsort (fun a b => a.2 > b.2)).toList

def main (args : List String) : IO UInt32 := do
  let manifest? := match args with
    | ["--manifest", f] => some f
    | [f] => some f
    | _ => none
  match manifest? with
  | none => IO.eprintln "oracle-wasm-diff: usage: oracle-wasm-diff (--manifest FILE | FILE)"; return 2
  | some manifest =>
    let dirs ← readManifest manifest
    let mut cases : List (String × Ast.Module × String × ByteArray) := []
    for dir in dirs do
      match ← loadCase dir with
      | some c => cases := c :: cases
      | none => pure ()
    -- Per-case verdicts (with the skip REASON + divergence outcomes) — the triage view: a skip tells you
    -- WHICH side declined and why (wasm .err/.unsupported vs a Core reduce gap), a diverge shows both sides.
    let mut tally : Tally := {}
    let mut skipReasons : List String := []
    for (id, coreAst, coreWat, rtBytes) in cases.reverse do
      match differential Oracle.Wasm.talosDriver coreAst coreWat rtBytes { entry := "main" } with
      | .agree => tally := { tally with agree := tally.agree + 1 }
                  IO.println s!"AGREE {id}"
      | .diverge core wasm => tally := { tally with diverge := tally.diverge + 1 }
                              -- Print the actual disagreement to STDOUT (survives the CI log; stderr is
                              -- truncated) so a single diverging sub-case is triageable from the log alone:
                              -- the case-dir path (holds program.ast/core.wat/core.ast), the Core reference
                              -- outcome (reduce core.ast), and the wasm outcome (runWasmWith talosDriver).
                              IO.println s!"DIVERGE {id}: core-ref = {outcomeStr core} | wasm = {outcomeStr wasm}"
      | .skip r => tally := { tally with skip := tally.skip + 1 }
                   skipReasons := r :: skipReasons
                   IO.println s!"SKIP {id}: {r}"
      | .leak n => tally := { tally with leak := tally.leak + 1 }
                   -- W6: values AGREE but the wasm run left `n` live heap objects at end-of-run — a Perceus
                   -- leak (an alloc never balanced by a drop). A distinct signal from diverge/skip.
                   IO.println s!"LEAK {id}: {n} live heap object(s) at end-of-run (Perceus leak)"
    IO.println s!"oracle-wasm-diff: {tally.agree} agree, {tally.diverge} diverge, {tally.skip} skip, {tally.leak} leak (of {cases.length} cases)"
    -- Skip-reason histogram (descending) — where the runnable fraction is lost; with v-wasm-oracle's
    -- head-tagged reason `… (head=<name>)` it surfaces which unmodeled result-type heads dominate.
    IO.println "oracle-wasm-diff skip-reason histogram:"
    for (reason, n) in tallyReasons skipReasons do
      IO.println s!"  {n}\t{reason}"
    -- talos DRIVES the wasm side now; scalar/arith import-free cases run, heap/runtime-import cases
    -- `.err`→`.unsupported`→`.skip` (sound gap). A DIVERGENCE (nonzero exit) is a real miscompile finding.
    return (if tally.diverge == 0 then 0 else 1)

/-! ### END-TO-END conformance WITNESSES — the differential wiring REAL talos to REAL Core `reduce`.
Proof-of-life for the operator's "assert compilation was correct ALL THE WAY THROUGH" direction: for a
scalar/arith program, Core `reduce coreAst` and talos-interpret(`coreWat`) produce the SAME value (`.agree`),
a mismatched wat is caught (`.diverge`), and talos ACTUALLY EVALUATES arithmetic (40+2 == the Core 42).
`native_decide` compiles + runs the talos small-step interpreter (as in `Oracle.Wasm.Talos`), so these are
exercised on every build; the axiom lives in the exe, NOT the `Oracle` lib (which stays clean for the
capstone). `e2eProg n` = the Core program `(do (def (main) n) (export main))` (`reduce` → `.value (.int n)`),
`e2eRtInt` = the `(result-type main Int)` section — mirrors `Oracle.WasmDiff`'s in-lib fixtures. -/
private def e2eProg (n : UInt8) : Ast.Module :=
  { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8,
                .intLit false .dec (ByteArray.mk #[n]), .name "export".toUTF8],
    nodes := #[.atom 1, .atom 2, .list #[1], .atom 3, .list #[0, 2, 3],
               .atom 4, .atom 2, .list #[5, 6], .atom 0, .list #[8, 4, 7]],
    root := 9 }
private def e2eRtInt : ByteArray :=
  Ast.encode { leaves := #[.name "result-type".toUTF8, .name "main".toUTF8, .name "Int".toUTF8],
               nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2]], root := 3 }

-- Core `(main = 5)` vs a wat returning `i64 5` → AGREE (a real talos run EQUALS the Core reference).
example : (differential Oracle.Wasm.talosDriver (e2eProg 5)
    "(module (func (export \"main\") (result i64) i64.const 5))" e2eRtInt { entry := "main" } == .agree) = true := by
  native_decide
-- Core `(main = 5)` vs a wat returning `i64 6` → DIVERGE — the miscompile signal, end-to-end.
example : (differential Oracle.Wasm.talosDriver (e2eProg 5)
    "(module (func (export \"main\") (result i64) i64.const 6))" e2eRtInt { entry := "main" }
    == .diverge (.value (.int 5)) (.value (.int 6))) = true := by
  native_decide
-- Core `(main = 42)` vs a wat that COMPUTES `40 + 2` → AGREE (talos actually evaluates the arithmetic).
example : (differential Oracle.Wasm.talosDriver (e2eProg 42)
    "(module (func (export \"main\") (result i64) (i64.add (i64.const 40) (i64.const 2))))"
    e2eRtInt { entry := "main" } == .agree) = true := by
  native_decide
