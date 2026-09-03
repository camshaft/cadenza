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
import Oracle.Wasm.HeapHost
import Interpreter.Wasm.SmallStep
import Interpreter.Wasm.Decoder.Wat
import Interpreter.Wasm.Host.Registry

namespace Oracle.Wasm

/-- The small-step fuel budget. Raised 1M → 8M after `06-numeric-model-1398` (a bounded 300k-iteration
countdown ≈ 2.7M small-steps) hit the old 1M cap and surfaced a FALSE divergence: the loop terminates and
matches Core, it just needs > 1M steps. 8M covers ~880k-iteration bounded loops with headroom so they run to
completion → AGREE; a genuinely huge/infinite loop still exhausts it and `toOutcome` SKIPs (sound — out-of-fuel
is inconclusive, never asserted as `.diverges`). Tune upward if the skip histogram shows more fuel-bound cases. -/
def talosDefaultFuel : Nat := 8000000

/-- Map a talos scalar `Wasm.Value` to the oracle's `WasmVal`. Integers take the SIGNED interpretation of
the wasm bits; floats keep raw IEEE bits. A non-scalar (ref/v128) result → `none` (surfaced as `.err`). -/
def talosToWasmVal : _root_.Wasm.Value → Option WasmVal
  | .i32 n => some (.i32 n.toInt32.toInt)
  | .i64 n => some (.i64 n.toInt64.toInt)
  | .f32 b => some (.f32 b)
  | .f64 b => some (.f64 b)
  | _ => none

/-- The heap-host registry: every modeled `"heap"` op (from `Oracle.Heap.heapHostOps`) keyed by its
`ImportDecl` — module `"heap"`, the op name, and the `HostFn`'s own declared core signature (talos resolves
the emitted `(type N)` import sig to the same params/results, so the decl matches). `HostRegistry.envFor`
walks a module's imports and resolves each against this; `covers` checks every import is claimed. -/
def heapRegistry : _root_.Wasm.HostRegistry Oracle.Heap.HeapState :=
  Oracle.Heap.heapHostOps.map fun (name, hf) =>
    { decl := { «module» := "heap", name := name, params := hf.params, results := hf.results }, fn := hf }

/-- The talos `Driver`: decode the core-module `.wat`, supply the modeled `"heap"` runtime imports via the
heap-host registry (declining only a module that imports an op we do NOT yet model — a sound skip), run the
entry via the small-step machine over the `HeapState` host, and map the outcome to a `WasmOutcome`. Pure
(talos's decode/run are `Except`/fuel-bounded), so `runWasmWith talosDriver …` is a provable Lean term for
the differential theorem. W5.1c is VALUE-ONLY — the leak dimension (reading the final `HeapState.liveCount`)
is W6, per v-lean-oracle's `WasmOutcome.leakCount` seam ruling. -/
def talosDriverWithFuel (fuel : Nat) : Driver := fun coreWat trial =>
  match _root_.Wasm.Decoder.Wat.decode coreWat with
  | .error e => .err s!"wat decode: {e}"
  | .ok m =>
    if !heapRegistry.covers m then
      .err "module imports an unmodeled runtime op (heap op not yet in the host)"
    else
      match m.findExport trial.entry with
      | none => .err s!"unknown export `{trial.entry}`"
      | some idx =>
        -- Supply the modeled heap ops as the module's host environment (positional over `m.imports`); the
        -- host STATE starts empty (`initialStore` seeds `host := default`, the empty `HeapState`).
        let host := heapRegistry.envFor m
        let store0 := m.runActiveSegments fuel (m.runConstGlobals fuel (m.initialStore (α := Oracle.Heap.HeapState)) host) host
        -- Zero-init the entry's params so a PARAM-TAKING `main` runs `f(0⃗)` instead of failing `local.get 0`
        -- on an unbound param slot (see W5.1a). The entry's unified index `idx` counts IMPORTS first, so the
        -- entry's own function is `m.funcs[idx - m.imports.length]` — with heap imports now present this
        -- offset matters (in W5.1a `imports.length` was 0). Passed REVERSED: talos binds
        -- `(args.take numParams).reverse` to local 0.. (params reversed on entry per the calling convention).
        -- v-lean-oracle's runCorpus applies Core `main` to the SAME typed zeros; a NON-SCALAR Core param
        -- makes the Core side SKIP, so a compound param never yields a false differential.
        let zeroArgs : List _root_.Wasm.Value :=
          (((m.funcs[idx - m.imports.length]?).map (fun fn => fn.params.map _root_.Wasm.ValueType.zero)).getD []).reverse
        match _root_.Wasm.SmallStep.initSingleModuleConfig m host idx store0 zeroArgs with
        | .error err => .err s!"small-step init: {err.message}"
        | .ok cfg =>
          match (_root_.Wasm.SmallStep.runSteps fuel cfg).result with
          | .success results finalStore =>
            match results.reverse.mapM talosToWasmVal with
            -- W6: carry the final heap-leak census (liveCount) on `.ok` (per v-lean-oracle's leakCount seam).
            | some vs => .ok vs.toArray finalStore.wasm.host.liveCount
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

/-- A PARAM-TAKING `main : (i64) -> i64` returning its param (identity). Proves the driver zero-inits the
entry's param slot: with no supplied arg it runs `main(0) = 0`, not a `local.get 0` failure (the dominant
skip cluster before this fix). -/
private def watIdI64 : String :=
  "(module (func (export \"main\") (param i64) (result i64) local.get 0))"
example : (talosDriver watIdI64 { entry := "main" } == .ok #[.i64 0]) = true := by native_decide

/-! ### W5.1c heap-host witnesses — a module importing `"heap"` ops now RUNS (not declined), proving the
registry pivot end-to-end: emitted import → resolved against `heapRegistry` → heap op executes → scalar
result. (Numeric func/local indices, matching `wasm-tools print` output on an unnamed emitted module.) -/

/-- Boxes 42, reads it back, drops it, returns the read value — importing `box-int` (call 0), `get-int`
(call 1), `drop` (call 2) from `"heap"`. With the heap host wired, `main() = 42` (balanced: box allocates,
drop frees, so no leak). Proves the registry resolves + the heap ops run + the scalar returns. -/
private def watHeapBoxGet : String :=
  "(module (import \"heap\" \"box-int\" (func (param i64) (result i32))) (import \"heap\" \"get-int\" (func (param i32) (result i64))) (import \"heap\" \"drop\" (func (param i32))) (func (export \"main\") (result i64) (local i32) (local i64) i64.const 42 call 0 local.set 0 local.get 0 call 1 local.set 1 local.get 0 call 2 local.get 1))"
example : (talosDriver watHeapBoxGet { entry := "main" } == .ok #[.i64 42]) = true := by native_decide

/-- A module importing an UNMODELED runtime op (`vec-push`, not yet in the host) declines to a sound skip
(`.err`), NOT a spurious run — the `covers` gate. -/
private def watHeapUnmodeled : String :=
  "(module (import \"heap\" \"vec-push\" (func (param i32 i32) (result i32))) (func (export \"main\") (result i64) i64.const 0))"
example : (match talosDriver watHeapUnmodeled { entry := "main" } with | .err _ => true | _ => false) = true := by native_decide

end Oracle.Wasm
