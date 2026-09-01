/-
The Core↔wasm DIFFERENTIAL GLUE (v-lean-oracle) — the wasm HALF of the operator's "assert compilation was
correct all the way through, not just to the Core layer" directive.

`Oracle.Wasm.runWasmWith` (v-wasm-oracle) runs an emitted core-module WAT behind an injectable `Driver`
(talos plugs in once the Lean-4.32.2 toolchain lands). THIS module is the glue that ties it to the Core
REFERENCE semantics (`Oracle.reduce` = `execute … #[]`): `differential` compares, for a program whose Core
AST is `coreAst` and whose compiled core-module WAT is `coreWat`,

    Core reference  `reduce coreAst`   vs   wasm run  `runWasmWith drive coreWat rtBytes trial`

and yields `agree` / `diverge` (a real MISCOMPILE — the two disagree on a concrete result) / `skip` (the
wasm side declined: a heap/collection/unmodeled case talos rejects, or a Core-side `unsupported` — a SOUND
coverage gap, NEVER a false differential). It is DRIVER-ABSTRACT (quantifies over `Driver`) so it lands and
gates on the current toolchain with NO talos dependency; the harness supplies `coreWat`/`rtBytes` (impure:
component-unbundle + `wasm-tools print` + the `cdz-result-type` section) and plugs `talosDriver` in for the
real run. See `Oracle.Wasm` for the wasm-side mapping and `WASM.md` for the pipeline.
-/
import Oracle.Wasm
import Oracle.Eval

namespace Oracle.WasmDiff

open Oracle Oracle.Wasm

/-- The differential verdict between the Core reference and the wasm run.
`agree` = same concrete result (value via `valueEqSpec`; both trap; both diverge). `diverge` = a real
disagreement (a miscompile candidate — carries both outcomes for the report). `skip` = one side is
`unsupported` (the wasm side declines heap/collection/import cases; the Core side has a modeling gap) — a
sound coverage gap, never reported as a divergence. -/
inductive Verdict where
  | agree
  | diverge (core wasm : Outcome)
  | skip (reason : String)
  deriving BEq, Inhabited

/-- Compare the Core reference (`reduce coreAst`) against the wasm run (`runWasmWith …`) for one trial.
Trap-vs-trap / diverges-vs-diverges agree WITHOUT comparing messages (the two interpreters word them
differently); value-vs-value uses `valueEqSpec` (float-aware: `NaN`/`-0.0` handled per spec). Any
`unsupported` (either side) → `skip`; any other cross-shape (e.g. Core `.value` vs wasm `.trap`) → `diverge`
(a genuine miscompile signal). `errReturn` never reaches a program's top-level outcome (the function boundary
converts it) — treated as `skip` defensively. -/
def differential (drive : Driver) (coreAst : Ast.Module) (coreWat : String)
    (rtBytes : ByteArray) (trial : Trial) : Verdict :=
  let core := Oracle.reduce coreAst
  let wasm := runWasmWith drive coreWat rtBytes trial
  match core, wasm with
  | _, .unsupported r => .skip r
  | .unsupported r, _ => .skip r
  | .value cv, .value wv => if Value.valueEqSpec cv wv then .agree else .diverge core wasm
  | .trap _, .trap _ => .agree
  | .diverges, .diverges => .agree
  | .errReturn _, _ | _, .errReturn _ => .skip "errReturn at boundary"
  | _, _ => .diverge core wasm

/-! ### Gate witnesses — the differential verdict logic, through a STUB driver + a real Core program +
a real `cdz-result-type` section round-trip. (No corpus case exercises this internal glue, so per
PRINCIPLES.md it belongs in Lean; the talos driver replaces the stub once the toolchain lands.) -/

/-- The `cdz-result-type` section bytes for `(result-type main <tyName>)` (a real `Oracle.Ast` encode). -/
private def rt (tyName : String) : ByteArray :=
  Ast.encode { leaves := #[.name "result-type".toUTF8, .name "main".toUTF8, .name tyName.toUTF8],
               nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2]], root := 3 }

/-- A Core program `(do (def (main) n) (export main))` — `reduce` gives `.value (.int n)`. -/
private def _progMain (n : UInt8) : Ast.Module :=
  { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8,
                .intLit false .dec (ByteArray.mk #[n]), .name "export".toUTF8],
    nodes := #[.atom 1, .atom 2, .list #[1], .atom 3, .list #[0, 2, 3],
               .atom 4, .atom 2, .list #[5, 6], .atom 0, .list #[8, 4, 7]],
    root := 9 }

-- Core reference `reduce (main = 5)` = `.value (.int 5)`; the stub wasm run returns i64 5 → AGREE.
#guard differential (fun _ _ => .ok #[.i64 5]) (_progMain 5) "(module)" (rt "Int") { entry := "main" } == .agree
-- wasm returns a DIFFERENT value (6 ≠ 5) → DIVERGE (the miscompile signal, both outcomes carried).
#guard differential (fun _ _ => .ok #[.i64 6]) (_progMain 5) "(module)" (rt "Int") { entry := "main" }
       == .diverge (.value (.int 5)) (.value (.int 6))
-- wasm TRAPs while Core produces a value → DIVERGE.
#guard differential (fun _ _ => .trap "unreachable") (_progMain 5) "(module)" (rt "Int") { entry := "main" }
       == .diverge (.value (.int 5)) (.trap "unreachable")
-- wasm side DECLINES (talos rejects imports on a heap case) → SKIP (sound coverage gap, not a divergence).
#guard differential (fun _ _ => .err "imports") (_progMain 5) "(module)" (rt "Int") { entry := "main" } == .skip "imports"
-- an unmodeled result-type spelling short-circuits inside runWasmWith → wasm `.unsupported` → SKIP.
#guard differential (fun _ _ => .ok #[.i64 5]) (_progMain 5) "(module)" (rt "Widget") { entry := "main" }
       == .skip "cdz-result-type: entry has no modeled scalar result type"

/-! ### The conformance RUNNER (v-lean-oracle owns this) — tally `differential` over a corpus of cases.
DRIVER-ABSTRACT: `talosDriver` (from `Oracle.Wasm.Talos`, once the co-land lands) plugs in as `drive`. The
IO wrapper (parse a manifest, read each case's coreWat/rtBytes/coreAst files, print the tally + divergence
report) is the executable `main`, added when talos is on main; this is its pure core. Each case is
`(id, coreAst, coreWat, rtBytes)`; a DIVERGENCE keeps `(id, coreOutcome, wasmOutcome)` for the report — a
divergence is a real MISCOMPILE FINDING to route to the compiler owners. -/
structure Tally where
  agree : Nat := 0
  diverge : Nat := 0
  skip : Nat := 0
  deriving Repr, BEq, Inhabited

/-- Run the differential over a corpus, tallying agree/diverge/skip + collecting divergence details
`(id, core, wasm)` (the miscompile findings). Driver-abstract. -/
def runCorpus (drive : Driver) (trial : Trial)
    (cases : List (String × Ast.Module × String × ByteArray)) :
    Tally × List (String × Outcome × Outcome) :=
  cases.foldl (fun (acc : Tally × List (String × Outcome × Outcome)) c =>
    let (t, divs) := acc
    match differential drive c.2.1 c.2.2.1 c.2.2.2 trial with
    | .agree => ({t with agree := t.agree + 1}, divs)
    | .diverge cv wv => ({t with diverge := t.diverge + 1}, (c.1, cv, wv) :: divs)
    | .skip _ => ({t with skip := t.skip + 1}, divs)) ({}, [])

-- runner tally over 3 stub cases: agree (5=5) / diverge (5≠9) / skip (unmodeled result type).
#guard (runCorpus (fun _ _ => .ok #[.i64 5]) { entry := "main" }
         [("a", _progMain 5, "(module)", rt "Int"), ("b", _progMain 9, "(module)", rt "Int"),
          ("c", _progMain 5, "(module)", rt "Widget")]).1 == { agree := 1, diverge := 1, skip := 1 }

end Oracle.WasmDiff
