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
  -- W6 — the Perceus LEAK dimension: the values agree but the wasm run left `count` live heap objects at
  -- end-of-run (`HeapState.liveCount > 0`), i.e. an alloc was never balanced by a drop. A distinct verdict
  -- from `diverge` (the VALUE is correct) and from `trap` (UAF/double-free already trap on the wasm side).
  | leak (count : Nat)
  deriving BEq, Inhabited

/-- The typed ZERO value for an ANNOTATED scalar param spec `(: name Ty)` (param-main zero-init, Option A —
coordinated with v-wasm-oracle #7819, which passes the matching wasm-valtype zeros): `Int*`/`UInt*` → `.int 0`,
`Bool` → `.bool false`, `Float32/64` → `.f64 0.0`. `none` for a bare/unannotated param or a non-scalar /
unmodeled annotation → the caller SKIPs the whole param-main (sound; both sides otherwise compute `f(0⃗)`). -/
def paramZero? (m : Ast.Module) (specId : Nat) : Option Value :=
  match m.nodes[specId]? with
  | some (Ast.Node.list pc) =>
    if m.headName? (Ast.Node.list pc) == some ":".toUTF8 then
      (match pc[2]? with
       | some tid =>
         (match Eval.parseIntTy? m tid with
          | some _ => some (.int 0)
          | none =>
            (match Eval.nameOf? m tid with
             | some nm => if nm == "Bool".toUTF8 then some (.bool false)
                          else if nm == "Float64".toUTF8 || nm == "Float32".toUTF8 then some (.f64 0.0)
                          else none
             | none => none))
       | none => none)
    else none
  | _ => none

/-- Compare a Core outcome to a wasm outcome (+ the wasm run's leak count) → a `Verdict`. Trap-vs-trap /
diverges-vs-diverges agree without comparing messages; value-vs-value uses `valueEqSpec` (float-aware).
Any `unsupported` (either side) → skip. 🔑 A `.diverges` is FUEL-EXHAUSTION (the Lean-oracle / talos step
bound), NOT a proof of non-termination — so an INCONCLUSIVE `.diverges` on ONE side vs a CONCRETE outcome
(value/trap) on the other is a SKIP, never a `.diverge` (else a fuel-heavy but terminating case
false-diverges vs a concrete wasm result — v-wasm 09-functions-0233). Both-`.diverges` = both inconclusive
→ agree (weakly consistent). `errReturn` → skip defensively. Any remaining cross-shape (Core `.value` vs
wasm `.trap`, etc.) → `.diverge` (a genuine miscompile signal). -/
def compareOutcomes (core wasm : Outcome) (leak : Nat) : Verdict :=
  match core, wasm with
  | _, .unsupported r => .skip r
  | .unsupported r, _ => .skip r
  | .value cv, .value wv =>
    if Value.valueEqSpec cv wv then (if leak > 0 then .leak leak else .agree) else .diverge core wasm
  | .trap _, .trap _ => .agree
  | .diverges, .diverges => .agree
  | .errReturn _, _ | _, .errReturn _ => .skip "errReturn at boundary"
  -- INCONCLUSIVE fuel-exhaustion on one side vs a concrete outcome → SKIP (not a false diverge):
  | .diverges, _ => .skip "core reduce hit its fuel/step limit — INCONCLUSIVE (a Lean-oracle bound, not a proven non-termination); cannot compare to a concrete wasm outcome"
  | _, .diverges => .skip "wasm run hit its fuel limit — inconclusive; cannot compare to a concrete Core outcome"
  | _, _ => .diverge core wasm

-- Core out-of-fuel `.diverges` vs a CONCRETE wasm outcome (value/trap) is INCONCLUSIVE → SKIP, never a
-- false diverge (v-wasm 09-functions-0233: a fuel-heavy arrays+vecs+division case whose Core reduce runs
-- out of fuel while the emitted wasm concretely traps div-by-zero — not a proven miscompile).
#guard (match compareOutcomes .diverges (.trap "div-by-zero") 0 with | .skip _ => true | _ => false)
#guard (match compareOutcomes (.value (.int 5)) .diverges 0 with | .skip _ => true | _ => false)  -- wasm out-of-fuel → skip
#guard compareOutcomes .diverges .diverges 0 == .agree                                             -- both inconclusive → agree (unchanged)
#guard compareOutcomes (.value (.int 5)) (.trap "unreachable") 0 == .diverge (.value (.int 5)) (.trap "unreachable")  -- concrete≠concrete → still DIVERGE

/-- Compare the Core reference (`reduce`/`execute` on `coreAst`) against the wasm run (`runWasmWith …`) for
one trial (via `compareOutcomes`). A NULLARY main → `reduce`; a scalar-param main → apply to typed zeros. -/
def differential (drive : Driver) (coreAst : Ast.Module) (coreWat : String)
    (rtBytes : ByteArray) (trial : Trial) : Verdict :=
  -- A NULLARY main → `reduce`. A PARAM-taking main → apply Core `main` to typed ZEROS matching its
  -- annotated scalar param types (v-wasm's driver passes the same wasm-valtype zeros), so both compute
  -- `f(0⃗)`. A bare/non-scalar param → `.unsupported` (skip) — zero-init is undefined there.
  let core : Outcome :=
    match Eval.mainParamsBody? coreAst with
    | some (specs, _) =>
      if specs.isEmpty then Oracle.reduce coreAst
      else (match specs.toList.mapM (paramZero? coreAst) with
            | some zeros => Oracle.execute coreAst zeros.toArray
            | none => .unsupported "wasm-diff: param-main with a bare / non-scalar param — zero-init skip")
    | none => Oracle.reduce coreAst
  let (wasm, leak) := runWasmWithLeak drive coreWat rtBytes trial
  compareOutcomes core wasm leak

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
-- W6: SAME value (5=5) but the wasm run left 2 live heap objects (leakCount 2) → LEAK 2 (value correct,
-- memory leaked — a Perceus violation, distinct from agree/diverge). leakCount 0 (the default above) → agree.
#guard differential (fun _ _ => .ok #[.i64 5] 2) (_progMain 5) "(module)" (rt "Int") { entry := "main" } == .leak 2
-- wasm returns a DIFFERENT value (6 ≠ 5) → DIVERGE (the miscompile signal, both outcomes carried).
#guard differential (fun _ _ => .ok #[.i64 6]) (_progMain 5) "(module)" (rt "Int") { entry := "main" }
       == .diverge (.value (.int 5)) (.value (.int 6))
-- A PARAM-taking main `(do (def (main (: x Int64)) x) (export main))`: the Core side applies `main` to a
-- typed ZERO (.int 0) → `main(0) = 0`; the stub wasm run returns i64 0 → AGREE (both compute f(0)).
private def _progParamMain : Ast.Module :=
  { leaves := #[.name "do".toUTF8, .name "def".toUTF8, .name "main".toUTF8, .name ":".toUTF8,
                .name "x".toUTF8, .name "Int64".toUTF8, .name "export".toUTF8],
    nodes := #[.atom 3, .atom 4, .atom 5, .list #[0, 1, 2], .atom 2, .list #[4, 3], .atom 4,
               .atom 1, .list #[7, 5, 6], .atom 6, .atom 2, .list #[9, 10], .atom 0, .list #[12, 8, 11]],
    root := 13 }
#guard differential (fun _ _ => .ok #[.i64 0]) _progParamMain "(module)" (rt "Int") { entry := "main" } == .agree
-- wasm TRAPs while Core produces a value → DIVERGE.
#guard differential (fun _ _ => .trap "unreachable") (_progMain 5) "(module)" (rt "Int") { entry := "main" }
       == .diverge (.value (.int 5)) (.trap "unreachable")
-- wasm side DECLINES (talos rejects imports on a heap case) → SKIP (sound coverage gap, not a divergence).
#guard differential (fun _ _ => .err "imports") (_progMain 5) "(module)" (rt "Int") { entry := "main" } == .skip "imports"
-- an unmodeled result-type spelling short-circuits inside runWasmWith → wasm `.unsupported` → SKIP.
-- (reason now carries the unresolved head tag, #7708 — the head-tagged reason feeds the skip histogram.)
#guard differential (fun _ _ => .ok #[.i64 5]) (_progMain 5) "(module)" (rt "Widget") { entry := "main" }
       == .skip "cdz-result-type: entry has no modeled scalar result type (head=Widget)"

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
  leak : Nat := 0          -- W6: value-agreeing runs that left live heap objects (a Perceus leak)
  capped : Nat := 0        -- a case whose differential exceeded the per-case WALL-CLOCK cap (--cap-ms):
                           -- a fuel-heavy near-runaway skipped so the shard completes (per-case time bound,
                           -- distinct from a fuel/step bound — sharding bounds count, not per-case time).
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
    | .skip _ => ({t with skip := t.skip + 1}, divs)
    | .leak _ => ({t with leak := t.leak + 1}, divs)) ({}, [])

-- runner tally over 3 stub cases: agree (5=5) / diverge (5≠9) / skip (unmodeled result type).
#guard (runCorpus (fun _ _ => .ok #[.i64 5]) { entry := "main" }
         [("a", _progMain 5, "(module)", rt "Int"), ("b", _progMain 9, "(module)", rt "Int"),
          ("c", _progMain 5, "(module)", rt "Widget")]).1 == { agree := 1, diverge := 1, skip := 1 }

end Oracle.WasmDiff
