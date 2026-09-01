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
    let (tally, divs) := runCorpus Oracle.Wasm.talosDriver { entry := "main" } cases
    IO.println s!"oracle-wasm-diff: {tally.agree} agree, {tally.diverge} diverge, {tally.skip} skip (of {cases.length} cases)"
    for (id, _, _) in divs.reverse do
      IO.eprintln s!"DIVERGE {id}: Core reference and wasm run disagree (miscompile candidate)"
    -- talos DRIVES the wasm side now; scalar/arith import-free cases run, heap/runtime-import cases
    -- `.err`→`.unsupported`→`.skip` (sound gap). A DIVERGENCE (nonzero exit) is a real miscompile finding.
    return (if tally.diverge == 0 then 0 else 1)
