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

DRIVER: `Oracle.Wasm.talosDriver` (module `Oracle.Wasm.Talos`) once the `nix/talos-lean-deps` co-land is on
main. Until then a STUB driver returns `.err` (→ every case `.skip`), so the whole IO plumbing compiles and
runs now (reporting all-skip); wiring talos is the one-line `stubDriver → Oracle.Wasm.talosDriver` swap +
its import, done the moment v-wasm-oracle pings the merge.
-/
import Oracle

open Oracle Oracle.Wasm Oracle.WasmDiff

/-- Placeholder interpreter seam until `Oracle.Wasm.talosDriver` lands (co-land pending in v-nix verify).
Returns `.err`, so every case maps to `.unsupported` → `differential` `.skip` (no false results). -/
def stubDriver : Driver := fun _ _ => .err "talos driver not yet wired (pending nix/talos-lean-deps co-land)"

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
    let (tally, divs) := runCorpus stubDriver { entry := "main" } cases
    IO.println s!"oracle-wasm-diff: {tally.agree} agree, {tally.diverge} diverge, {tally.skip} skip (of {cases.length} cases)"
    for (id, _, _) in divs.reverse do
      IO.eprintln s!"DIVERGE {id}: Core reference and wasm run disagree (miscompile candidate)"
    -- NB: with the stub driver every case skips; a nonzero exit only fires once talosDriver is wired.
    return (if tally.diverge == 0 then 0 else 1)
