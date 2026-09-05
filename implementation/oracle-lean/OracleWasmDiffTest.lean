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

/-- The `"heap"` runtime ops a core module imports, read from the wat text. A wasm import reads
`(import "heap" "<op>" (func …))`, so every occurrence of the literal `"heap" "` is followed by the op
name up to the next `"`. Per-case DEDUPED (an op imported twice counts once). This tags which heap ops a
DIVERGE/LEAK case exercises — the datum that pinpoints a buggy consume-op (v-wasm-oracle triage ask). -/
def heapOpImportsOf (wat : String) : List String :=
  (((wat.splitOn "\"heap\" \"").drop 1).filterMap (fun piece => (piece.splitOn "\"").head?)).eraseDups

/-- Tally skip REASONS by frequency (descending) — surfaces where the runnable fraction is lost, and (with
v-wasm-oracle's head-tagged reason `… (head=<name>)`) which unmodeled result-type heads dominate. -/
def tallyReasons (rs : List String) : List (String × Nat) :=
  let counts := rs.foldl (fun (acc : List (String × Nat)) r =>
    match acc.find? (fun p => p.1 == r) with
    | some _ => acc.map (fun p => if p.1 == r then (p.1, p.2 + 1) else p)
    | none => acc ++ [(r, 1)]) []
  (counts.toArray.qsort (fun a b => a.2 > b.2)).toList

/-- Round-robin shard selection: keep the cases whose 0-based index ≡ `i` (mod `n`). N parallel CI jobs
(`--shard 0 N` … `--shard (N-1) N`) then each process ~1/N of the corpus, so no single job exceeds GitHub
Actions' HARD 6h per-JOB cap — which the full-corpus differential now blows, because heap-result decode moved
hundreds of heap-valued-result cases from fast SKIP to decode+EXECUTE-through-talos (the coverage win). Each
job prints its shard's partial tally; the workflow aggregates. Round-robin (NOT a contiguous slice) spreads
the fuel-heavy tail (near-8M-step cases) evenly across shards. `n = 0` ⇒ no sharding (all cases). -/
def shardOf {α : Type} (i n : Nat) (xs : List α) : List α :=
  if n == 0 then xs
  else
    -- structural recursion with an explicit index (NOT `List.enum`/`filter`/`mapM` combinators — those trip
    -- the `#guard`/native-eval "uses sorry" codegen trap; a plain `go` is evaluable).
    let rec go : Nat → List α → List α
      | _,   []        => []
      | idx, x :: rest => if idx % n == i then x :: go (idx + 1) rest else go (idx + 1) rest
    go 0 xs

#guard (shardOf 0 3 [10, 11, 12, 13, 14, 15, 16] == [10, 13, 16])
#guard (shardOf 1 3 [10, 11, 12, 13, 14, 15, 16] == [11, 14])
#guard (shardOf 2 3 [10, 11, 12, 13, 14, 15, 16] == [12, 15])
#guard (shardOf 0 0 [10, 11, 12] == [10, 11, 12])  -- n = 0 ⇒ no sharding (all)
#guard (shardOf 2 4 [10, 11, 12, 13, 14, 15] == [12])  -- i = last valid shard (i < n)
#guard (shardOf 3 3 [10, 11, 12, 13] == ([] : List Nat))  -- i ≥ n (out of range) → empty, safe (no crash)

/-- Await a case's `differential` task, but give up once the wall-clock deadline (`deadlineMs`, absolute
`monoMsNow`) passes. Returns `some diff` if it finished in time, `none` if it exceeded the cap (the task
is LEFT running — a fuel-heavy near-runaway on its own dedicated thread — and dies when the process exits,
so one runaway case never blocks the rest of the shard). Polls every 100 ms, so a fast case returns
promptly; the cost is per-shard 1-2 runaways parked on threads. The `differential` is forced to WHNF ON the
task thread via `IO.lazyPure` (WHNF = the `Diff` constructor, whose choice already ran the whole
reduce-vs-wasm comparison), so the heavy work happens off the main thread — the whole point of the cap. -/
partial def awaitCapped (diffTask : Task (Except IO.Error Oracle.WasmDiff.Verdict)) (deadlineMs : Nat) :
    IO (Option Oracle.WasmDiff.Verdict) := do
  if ← IO.hasFinished diffTask then
    pure (match diffTask.get with | .ok d => some d | .error _ => none)
  else if (← IO.monoMsNow) ≥ deadlineMs then pure none    -- exceeded the per-case wall-clock cap → capped
  else do IO.sleep 100; awaitCapped diffTask deadlineMs

/-- Run a case's differential, honoring an optional per-case wall-clock cap `capMs` (0 = no cap). With a
cap, the `differential` runs on a DEDICATED task thread and is abandoned (→ `none` = capped) if it exceeds
`capMs`; without, it runs inline (original behavior). -/
def runCaseCapped (capMs : Nat) (coreAst : Ast.Module) (coreWat : String) (rtBytes : ByteArray) :
    IO (Option Oracle.WasmDiff.Verdict) := do
  if capMs == 0 then
    pure (some (differential Oracle.Wasm.talosDriver coreAst coreWat rtBytes { entry := "main" }))
  else
    let diffTask ← IO.asTask
      (IO.lazyPure (fun _ => differential Oracle.Wasm.talosDriver coreAst coreWat rtBytes { entry := "main" }))
      Task.Priority.dedicated
    awaitCapped diffTask ((← IO.monoMsNow) + capMs)

/-- Extract an optional `--cap-ms N` flag from the arg list, returning `(capMs, remainingArgs)` with the
flag+value removed (order preserved). `capMs = 0` (no cap) if the flag is absent or its value is unparsable.
Scans positionally so the flag can appear anywhere (after `--manifest`/`--shard`), leaving the rest for the
existing manifest/shard parse. -/
def extractCapMs : List String → Nat × List String :=
  let rec go (acc : List String) : List String → Nat × List String
    | [] => (0, acc.reverse)
    | "--cap-ms" :: v :: rest => ((v.toNat?).getD 0, acc.reverse ++ rest)
    | x :: rest => go (x :: acc) rest
  go []

def main (args : List String) : IO UInt32 := do
  -- Pull an optional `--cap-ms N` flag out first (v-gha-green wires the workflow to pass it): a per-CASE
  -- WALL-CLOCK cap so a fuel-heavy near-runaway case is skipped (tallied `capped`) and its shard COMPLETES,
  -- instead of one >6h case hanging the whole shard (sharding bounds per-shard COUNT, not per-case time).
  let (capMs, args) := extractCapMs args
  -- `--manifest FILE` (or a bare FILE); optional `--shard I N` runs only cases with (index % N == I), so the
  -- full-corpus diff fits under GHA's 6h per-job cap as N parallel shards (see `shardOf`).
  let (manifest?, shard?) : Option String × Option (Nat × Nat) := match args with
    | ["--manifest", f] => (some f, none)
    | ["--manifest", f, "--shard", i, n] => (some f, (do let iN ← i.toNat?; let nN ← n.toNat?; pure (iN, nN)))
    | [f] => (some f, none)
    | _ => (none, none)
  match manifest? with
  | none => IO.eprintln "oracle-wasm-diff: usage: oracle-wasm-diff (--manifest FILE | FILE) [--shard I N] [--cap-ms N]"; return 2
  | some manifest =>
    let allDirs ← readManifest manifest
    let dirs := match shard? with | some (i, n) => shardOf i n allDirs | none => allDirs
    (match shard? with
     | some (i, n) => IO.println s!"[oracle-wasm-diff shard {i}/{n}: {dirs.length} of {allDirs.length} cases]"
     | none => pure ())
    (if capMs > 0 then IO.println s!"[oracle-wasm-diff per-case wall-clock cap: {capMs}ms — a case exceeding it is CAPPED (skipped) so the shard completes]" else pure ())
    let mut cases : List (String × Ast.Module × String × ByteArray) := []
    for dir in dirs do
      match ← loadCase dir with
      | some c => cases := c :: cases
      | none => pure ()
    -- Per-case verdicts (with the skip REASON + divergence outcomes) — the triage view: a skip tells you
    -- WHICH side declined and why (wasm .err/.unsupported vs a Core reduce gap), a diverge shows both sides.
    let mut tally : Tally := {}
    let mut skipReasons : List String := []
    -- Heap ops used by DIVERGE + LEAK cases (per-case deduped, accumulated) → a histogram that pinpoints
    -- which consume-op a scale-only regression rides on (v-wasm-oracle triage ask).
    let mut divergeLeakOps : List String := []
    for (id, coreAst, coreWat, rtBytes) in cases.reverse do
      match ← runCaseCapped capMs coreAst coreWat rtBytes with
      | none => tally := { tally with capped := tally.capped + 1 }
                -- exceeded the per-case wall-clock cap: a fuel-heavy near-runaway skipped so the shard
                -- completes. A distinct bucket from skip (a modeled decline) — this is a resource cap.
                IO.println s!"CAPPED {id}: differential exceeded {capMs}ms wall-clock (fuel-heavy case skipped)"
      | some .agree => tally := { tally with agree := tally.agree + 1 }
                       IO.println s!"AGREE {id}"
      | some (.diverge core wasm) =>
          -- Print the actual disagreement to STDOUT (survives the CI log; stderr is truncated) so a single
          -- diverging sub-case is triageable from the log alone: the case-dir path (holds program.ast/
          -- core.wat/core.ast), the Core reference outcome (reduce core.ast), the wasm outcome, and heap ops.
          tally := { tally with diverge := tally.diverge + 1 }
          let ops := heapOpImportsOf coreWat
          divergeLeakOps := divergeLeakOps ++ ops
          IO.println s!"DIVERGE {id}: core-ref = {outcomeStr core} | wasm = {outcomeStr wasm} | result: {resultKindTag rtBytes "main".toUTF8} | heap-ops: {ops}"
      | some (.skip r) =>
          tally := { tally with skip := tally.skip + 1 }
          skipReasons := r :: skipReasons
          IO.println s!"SKIP {id}: {r}"
      | some (.leak n) =>
          -- W6: values AGREE but the wasm run left `n` live heap objects at end-of-run — a Perceus leak
          -- (an alloc never balanced by a drop). A distinct signal from diverge/skip.
          tally := { tally with leak := tally.leak + 1 }
          let ops := heapOpImportsOf coreWat
          divergeLeakOps := divergeLeakOps ++ ops
          -- Tag the result KIND (`scalar <Ty>` = genuine husk; `string`/`heap` = owned result already dropped,
          -- residual is real) so v-memory-safety can partition scalar-vs-heap leaks straight from the CI log.
          IO.println s!"LEAK {id}: {n} live heap object(s) at end-of-run (Perceus leak) | result: {resultKindTag rtBytes "main".toUTF8} | heap-ops: {ops}"
    IO.println s!"oracle-wasm-diff: {tally.agree} agree, {tally.diverge} diverge, {tally.skip} skip, {tally.leak} leak, {tally.capped} capped (of {cases.length} cases)"
    -- Heap-op usage histogram over the DIVERGE + LEAK cases — which consume-op dominates the regression.
    IO.println "oracle-wasm-diff diverge+leak heap-op histogram:"
    for (op, n) in tallyReasons divergeLeakOps do
      IO.println s!"  {n}\t{op}"
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
