/-
The talos `Driver` — fills the `Oracle.Wasm.Driver` seam by driving the talos wasm interpreter
(`cajal-technologies/talos`) over the rcdzc-emitted core-module `.wat`. This is the W2/W3 slice.

Imports ONLY talos's EXECUTION modules (`Interpreter.Wasm.SmallStep` + `Interpreter.Wasm.Decoder.Wat`),
whose transitive closure is Std-only — NOT the umbrella `Interpreter.Wasm` (which re-exports the Mathlib-heavy
Wp/proof layer). So this builds Mathlib-free (verified: the exec closure compiles, mathlib oleans = 0).

The adapter replicates talos's own `runner` invocation (α := Unit ⇒ empty host, matching the self-contained,
zero-import scalar/arith subset; a runtime-importing module → `.err` → `.unsupported`, deferred to the heap
host increment). Talos value → `WasmVal` uses the SIGNED reading (`toInt32`/`toInt64`), matching
`Runner.renderValue`.
-/
import Oracle.Wasm
import Interpreter.Wasm.SmallStep
import Interpreter.Wasm.Decoder.Wat

namespace Oracle.Wasm

/-- A generous default fuel budget (matches talos's runner default). -/
def talosDefaultFuel : Nat := 1000000

/-- Map a talos scalar `Wasm.Value` to the oracle's `WasmVal`. Integers take the SIGNED interpretation of
the wasm bits; floats keep raw IEEE bits. A non-scalar (ref/v128) result → `none` (surfaced as `.err`). -/
def talosToWasmVal : _root_.Wasm.Value → Option WasmVal
  | .i32 n => some (.i32 n.toInt32.toInt)
  | .i64 n => some (.i64 n.toInt64.toInt)
  | .f32 b => some (.f32 b)
  | .f64 b => some (.f64 b)
  | _ => none

/-- The talos `Driver`: decode the core-module `.wat`, reject imports, run the entry via the small-step
machine, and map the outcome to a `WasmOutcome`. Pure (talos's decode/run are `Except`/fuel-bounded), so
`runWasmWith talosDriver …` is a provable Lean term for the differential theorem. -/
def talosDriverWithFuel (fuel : Nat) : Driver := fun coreWat trial =>
  match _root_.Wasm.Decoder.Wat.decode coreWat with
  | .error e => .err s!"wat decode: {e}"
  | .ok m =>
    if m.imports.length > 0 then
      .err "module declares imports (runtime-importing case not yet modeled)"
    else
      match m.findExport trial.entry with
      | none => .err s!"unknown export `{trial.entry}`"
      | some idx =>
        let store0 := m.runActiveSegments fuel (m.runConstGlobals fuel (m.initialStore (α := Unit)) {}) {}
        let inst : _root_.Wasm.SmallStep.ModuleInstance Unit := { module := m, host := {} }
        match _root_.Wasm.SmallStep.initConfig inst idx store0 [] with
        | .error err => .err s!"small-step init: {err.message}"
        | .ok cfg =>
          match (_root_.Wasm.SmallStep.runSteps fuel cfg).result with
          | .success results _ =>
            match results.reverse.mapM talosToWasmVal with
            | some vs => .ok vs.toArray
            | none => .err "non-scalar wasm result"
          | .trapped reason _ => .trap reason.message
          | .outOfFuel _ => .outOfFuel
          | .internalError err _ => .err s!"small-step internal: {err.message}"

/-- The talos `Driver` with the default fuel budget — the seam value `runWasmWith` consumes. -/
def talosDriver : Driver := talosDriverWithFuel talosDefaultFuel

/-! ### End-to-end gate witness — talos actually RUNS (compiled = the whole pipeline is exercised every build).
This is the one witness no corpus case can stand in for: it proves talos executes a real core module through
the `Driver` + boundary. (`native_decide` compiles + runs the interpreter; the module is tiny.) -/

/-- A hand-written zero-import scalar core module (`main : () -> i64` returning 5). -/
private def wat5 : String := "(module (func (export \"main\") (result i64) i64.const 5))"

-- talos runs `main` → the raw result stack is a single signed i64 5
example : (talosDriver wat5 { entry := "main" } == .ok #[.i64 5]) = true := by native_decide
-- full run_wasm boundary: through the driver + an `Int` result-type section → Outcome.value (int 5)
example :
    (runWasmWith talosDriver wat5
      (Ast.encode { leaves := #[.name "result-type".toUTF8, .name "main".toUTF8, .name "Int".toUTF8],
                    nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2]], root := 3 })
      { entry := "main" } == .value (.int 5)) = true := by native_decide
-- an unknown export declines (not a differential mismatch)
example : (talosDriver wat5 { entry := "nope" } == .err "unknown export `nope`") = true := by native_decide

end Oracle.Wasm
