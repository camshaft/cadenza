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
import Oracle.Wasm.HeapDecode
import Oracle.Eval
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
        -- Run the module's `(start $f)` BEFORE the entry. The emitted COMPONENT instantiates the core module —
        -- which runs its start, and the start BUILDS the program's constant/static compound data (a literal
        -- `{1,2,3}` set, a `(map …)`, …) via the heap ops into a global — and only THEN calls `main`. Extracting
        -- the bare core module and calling `main` directly would SKIP start, leaving that global at 0, so `main`
        -- reads handle 0 and the op traps `unknown handle 0` while Core produced a value — a FALSE divergence
        -- (the dominant 76-diverge cluster). Per the wasm spec, start runs at instantiation before any export,
        -- so running it here is the conformant instantiation. A start trap/out-of-fuel falls back to `store0`
        -- (rare; the entry run then surfaces the failure rather than crashing the driver).
        let store1 :=
          match m.startFunc with
          | none => store0
          | some sidx =>
            match _root_.Wasm.SmallStep.initSingleModuleConfig m host sidx store0 [] with
            | .error _  => store0
            | .ok scfg  =>
              match (_root_.Wasm.SmallStep.runSteps fuel scfg).result with
              | .success _ fin => fin.wasm
              | _              => store0
        -- Zero-init the entry's params so a PARAM-TAKING `main` runs `f(0⃗)` instead of failing `local.get 0`
        -- on an unbound param slot (see W5.1a). The entry's unified index `idx` counts IMPORTS first, so the
        -- entry's own function is `m.funcs[idx - m.imports.length]` — with heap imports now present this
        -- offset matters (in W5.1a `imports.length` was 0). Passed REVERSED: talos binds
        -- `(args.take numParams).reverse` to local 0.. (params reversed on entry per the calling convention).
        -- v-lean-oracle's runCorpus applies Core `main` to the SAME typed zeros; a NON-SCALAR Core param
        -- makes the Core side SKIP, so a compound param never yields a false differential.
        let zeroArgs : List _root_.Wasm.Value :=
          (((m.funcs[idx - m.imports.length]?).map (fun fn => fn.params.map _root_.Wasm.ValueType.zero)).getD []).reverse
        match _root_.Wasm.SmallStep.initSingleModuleConfig m host idx store1 zeroArgs with
        | .error err => .err s!"small-step init: {err.message}"
        | .ok cfg =>
          match (_root_.Wasm.SmallStep.runSteps fuel cfg).result with
          | .success results finalStore =>
            let host := finalStore.wasm.host
            -- W6: carry the final heap-leak census (liveCount) on `.ok` (per v-lean-oracle's leakCount seam).
            let scalarMap : WasmOutcome :=
              match results.reverse.mapM talosToWasmVal with
              | some vs => .ok vs.toArray host.liveCount
              | none    => .err "non-scalar wasm result"
            -- HEAP-valued result: a single i32 that is a live HEAP OBJECT (getObj? some) is a returned handle
            -- → decode it structurally from the final HeapState → `.compound`. A raw scalar / immediate result
            -- has `getObj? = none` (or isn't a lone i32) → the scalar map. An undecodable heap object (a sum,
            -- which declines) also falls through to the scalar map (and its result-type isn't heap-decodable, so
            -- `runWasmWithLeak` skips it anyway).
            match results.reverse with
            | [.i32 rawh] =>
              match host.getObj? rawh with
              | some _ =>
                match host.decodeValue? rawh with
                -- The returned handle is the RESULT (legitimately live — the component lift consumes it), so it
                -- must NOT count as a leak: drop it (cascades into its children) and the REMAINING live count is
                -- the actual leak census. A clean heap-valued run → 0; anything else still live → a real leak.
                -- Canonicalize the decoded value (sort/dedupe set/map/record via cmpValue) so it matches Core's
                -- order-sensitive `valueEqSpec` (a no-op for scalars/bigint/tuple/list). Eval.canonicalizeValue
                -- (v-lean-oracle, on main).
                | some v => .ok #[.compound (Eval.canonicalizeValue v)] (host.dropH rawh).liveCount
                | none   => scalarMap
              | none => scalarMap
            | _ => scalarMap
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

/-- A module importing an UNMODELED runtime op (`hash-blake3`, a W5.5 transport op not yet in the host)
declines to a sound skip (`.err`), NOT a spurious run — the `covers` gate. (Repoint to a still-unmodeled op
if this one ever gets modeled.) -/
private def watHeapUnmodeled : String :=
  "(module (import \"heap\" \"hash-blake3\" (func (param i32) (result i32))) (func (export \"main\") (result i64) i64.const 0))"
/-! ### End-to-end HEAP-ALLOC + LEAK-CENSUS witnesses — a module that actually ALLOCATES a heap object
(`map-empty` boxes a real map node) run through the full talos pipeline, proving the W6 leak dimension
end-to-end (not just at the pure `HeapState` layer). `watHeapBoxGet` above never allocates (box-int 42 is a
FIXNUM IMMEDIATE), so these are the first witnesses exercising alloc + the `leakCount` surfaced on `.ok`. Both
build the map `{7 to 99}` with immediate int key+value (census-excluded), read 99 back, and return it; they
differ only in whether the map node is dropped. -/

/-- BALANCED: build `{7 to 99}`, look up key 7, read 99, then DROP the map → `main() = 99` with `leakCount 0`
(the map node freed; immediates never allocated). Proves alloc+drop nets to a clean census through talos. -/
private def watHeapMap : String :=
  "(module (import \"heap\" \"map-empty\" (func (result i32))) (import \"heap\" \"box-int\" (func (param i64) (result i32))) (import \"heap\" \"map-insert\" (func (param i32) (param i32) (param i32) (result i32))) (import \"heap\" \"map-lookup\" (func (param i32) (param i32) (result i32))) (import \"heap\" \"get-int\" (func (param i32) (result i64))) (import \"heap\" \"drop\" (func (param i32))) (func (export \"main\") (result i64) (local i32) (local i32) (local i32) (local i32) (local i64) call 0 local.set 0 i64.const 7 call 1 local.set 1 i64.const 99 call 1 local.set 2 local.get 0 local.get 1 local.get 2 call 2 local.set 0 i64.const 7 call 1 local.set 1 local.get 0 local.get 1 call 3 local.set 3 local.get 3 call 4 local.set 4 local.get 0 call 5 local.get 4))"
example : (talosDriver watHeapMap { entry := "main" } == .ok #[.i64 99]) = true := by native_decide

/-- LEAKED: identical but the map node is NEVER dropped → `main() = 99` with `leakCount 1` (the map node
leaks). Proves the end-to-end leak census actually FIRES through talos — the dynamic Perceus witness that a
missing drop is caught (a real emit that leaks would surface here as `leakCount > 0` on a value-agreeing run). -/
private def watHeapMapLeak : String :=
  "(module (import \"heap\" \"map-empty\" (func (result i32))) (import \"heap\" \"box-int\" (func (param i64) (result i32))) (import \"heap\" \"map-insert\" (func (param i32) (param i32) (param i32) (result i32))) (import \"heap\" \"map-lookup\" (func (param i32) (param i32) (result i32))) (import \"heap\" \"get-int\" (func (param i32) (result i64))) (import \"heap\" \"drop\" (func (param i32))) (func (export \"main\") (result i64) (local i32) (local i32) (local i32) (local i32) (local i64) call 0 local.set 0 i64.const 7 call 1 local.set 1 i64.const 99 call 1 local.set 2 local.get 0 local.get 1 local.get 2 call 2 local.set 0 i64.const 7 call 1 local.set 1 local.get 0 local.get 1 call 3 local.set 3 local.get 3 call 4 local.set 4 local.get 4))"
example : (talosDriver watHeapMapLeak { entry := "main" } == .ok #[.i64 99] 1) = true := by native_decide

/-- End-to-end SET CONSUME path: build `{1,2}` and `{2,3}` (immediate int elems), `set-union` them (CONSUMES
both) into `{1,2,3}`, return `set-size` = 3, then DROP the union → `main() = 3` with `leakCount 0` (all three
set nodes freed — the two inputs by the union's consume, the result by the drop; immediate elems are census-
free). Complements `watHeapMap` (the map path): proves the set consume ops + the leak census balance through
talos, not just the pure `HeapState` layer. -/
private def watHeapSetUnion : String :=
  "(module (import \"heap\" \"set-empty\" (func (result i32))) (import \"heap\" \"box-int\" (func (param i64) (result i32))) (import \"heap\" \"set-insert\" (func (param i32) (param i32) (result i32))) (import \"heap\" \"set-union\" (func (param i32) (param i32) (result i32))) (import \"heap\" \"set-size\" (func (param i32) (result i32))) (import \"heap\" \"drop\" (func (param i32))) (func (export \"main\") (result i32) (local i32) (local i32) (local i32) call 0 i64.const 1 call 1 call 2 i64.const 2 call 1 call 2 local.set 0 call 0 i64.const 2 call 1 call 2 i64.const 3 call 1 call 2 local.set 1 local.get 0 local.get 1 call 3 local.set 0 local.get 0 call 4 local.set 2 local.get 0 call 5 local.get 2))"
example : (talosDriver watHeapSetUnion { entry := "main" } == .ok #[.i32 3]) = true := by native_decide

/-- End-to-end USE-AFTER-FREE trap: box a heap float, DROP it, then `get-float` the freed handle → the host
traps (UAF), talos surfaces it, and the run is `.trap` — NOT a value. Completes the end-to-end Perceus pair:
`watHeapMapLeak` proves a MISSING drop is caught (leakCount > 0), this proves a drop-too-early / double-free is
caught (a trap). A real emit with either bug surfaces here, not just at the pure `HeapState` layer. (box-float,
not box-int: a fixnum would inline as an immediate whose drop is a no-op, so no freed handle to trap on.) -/
private def watHeapUseAfterFree : String :=
  "(module (import \"heap\" \"box-float\" (func (param f64) (result i32))) (import \"heap\" \"drop\" (func (param i32))) (import \"heap\" \"get-float\" (func (param i32) (result f64))) (func (export \"main\") (result f64) (local i32) f64.const 1 call 0 local.set 0 local.get 0 call 1 local.get 0 call 2))"
example : (match talosDriver watHeapUseAfterFree { entry := "main" } with | .trap _ => true | _ => false) = true := by native_decide

/-- End-to-end PERCEUS REUSE (FBIP): build `[5]` (an array with an immediate int elem), `reset` it (UNIQUE
rc==1 → same-handle empty shell — the reuse token), `arr-alloc-reuse(2, token)` refits THAT shell to a 2-slot
array (zero alloc), read `arr-len` = 2, drop → `main() = 2` with `leakCount 0`. Proves the rc==1 in-place
reuse path (reset + arr-alloc-reuse) runs end-to-end through talos and leak-balances — the W7-crux ops are not
in the in-progress full-corpus run (which predates them), so this is their first end-to-end validation. -/
private def watHeapReuse : String :=
  "(module (import \"heap\" \"arr-alloc\" (func (param i32) (result i32))) (import \"heap\" \"box-int\" (func (param i64) (result i32))) (import \"heap\" \"arr-set\" (func (param i32) (param i32) (param i32) (result i32))) (import \"heap\" \"reset\" (func (param i32) (result i32))) (import \"heap\" \"arr-alloc-reuse\" (func (param i32) (param i32) (result i32))) (import \"heap\" \"arr-len\" (func (param i32) (result i32))) (import \"heap\" \"drop\" (func (param i32))) (func (export \"main\") (result i32) (local i32) (local i32) (local i32) i32.const 1 call 0 local.set 0 i64.const 5 call 1 local.set 1 local.get 0 i32.const 0 local.get 1 call 2 local.set 0 local.get 0 call 3 local.set 0 i32.const 2 local.get 0 call 4 local.set 0 local.get 0 call 5 local.set 2 local.get 0 call 6 local.get 2))"
example : (talosDriver watHeapReuse { entry := "main" } == .ok #[.i32 2]) = true := by native_decide

/-- End-to-end BORROW-heavy numeric (BigInt): `bigint-of-i64 2` and `bigint-of-i64 3`, `bigint-add` (BORROWS
both — a, b survive), `bigint-to-i64-checked` reads 5; the caller then drops a, b AND the sum → `main() = 5`
with `leakCount 0`. Complements the consume-semantics witnesses (map/set/reuse) by exercising the OPPOSITE
ownership discipline — the operands survive the op and are dropped by the caller — end-to-end through talos.
(The numeric ops are not in the in-progress full-corpus run, so this is their first end-to-end validation.) -/
private def watHeapBigInt : String :=
  "(module (import \"heap\" \"bigint-of-i64\" (func (param i64) (result i32))) (import \"heap\" \"bigint-add\" (func (param i32) (param i32) (result i32))) (import \"heap\" \"bigint-to-i64-checked\" (func (param i32) (result i64))) (import \"heap\" \"drop\" (func (param i32))) (func (export \"main\") (result i64) (local i32) (local i32) (local i32) (local i64) i64.const 2 call 0 local.set 0 i64.const 3 call 0 local.set 1 local.get 0 local.get 1 call 1 local.set 2 local.get 2 call 2 local.set 3 local.get 0 call 3 local.get 1 call 3 local.get 2 call 3 local.get 3))"
example : (talosDriver watHeapBigInt { entry := "main" } == .ok #[.i64 5]) = true := by native_decide

/-- End-to-end HEAP-VALUED RESULT decode: `main` builds a BigInt (of-i64 1000000000, a HEAP leaf) and RETURNS
its handle. The driver detects the lone-i32-heap-object result, decodes it from the final `HeapState` →
`WasmVal.compound (.int 1000000000)`, and reports `leakCount 0` (the returned handle is the RESULT — dropped
for the census, not a leak). This is the read+driver half of the heap-result lever, end-to-end through talos;
`runWasmWith … (rtBytes "BigInt")` then maps it to `.value (.int …)` (witnessed in `Oracle.Wasm`). -/
private def watHeapBigIntResult : String :=
  "(module (import \"heap\" \"bigint-of-i64\" (func (param i64) (result i32))) (func (export \"main\") (result i32) i64.const 1000000000 call 0))"
example : (talosDriver watHeapBigIntResult { entry := "main" } == .ok #[.compound (.int 1000000000)]) = true := by native_decide

/-- End-to-end COMPOUND heap-valued result + CANONICALIZE: `main` builds the set `{2,1}` (inserting 2 THEN 1,
i.e. out of canonical order) and returns it. The driver decodes the returned set handle → `Value.set` in
INSERTION order `[2,1]`, then `Eval.canonicalizeValue` sorts it → `[1,2]` = Core's canonical form (so the
order-sensitive `valueEqSpec` matches). `leakCount 0` (the returned set is the result — dropped for the census;
immediate elems are census-free). Exercises the decode + canonicalize of a COMPOUND result end-to-end. -/
private def watHeapSetResult : String :=
  "(module (import \"heap\" \"set-empty\" (func (result i32))) (import \"heap\" \"box-int\" (func (param i64) (result i32))) (import \"heap\" \"set-insert\" (func (param i32) (param i32) (result i32))) (func (export \"main\") (result i32) call 0 i64.const 2 call 1 call 2 i64.const 1 call 1 call 2))"
example : (talosDriver watHeapSetResult { entry := "main" } == .ok #[.compound (.set #[.int 1, .int 2])]) = true := by native_decide

end Oracle.Wasm
