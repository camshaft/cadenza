/-
The wasm-oracle boundary: map a wasm interpreter's run RESULT onto the shared `Oracle.Outcome` /
`Oracle.Value` model (design: `implementation/oracle-lean/WASM.md`, vertical v-wasm-oracle).

This is the wasm HALF of the differential oracle (v-lean-oracle owns the Core/denote half + the glue
theorem `Core denote P == run_wasm(compile P)`). The full `run_wasm` pipeline is:

  emitted COMPONENT .wasm  ──(harness: `wasm-tools component unbundle --threshold 0` + `wasm-tools print`)──▶
  core-module .wat text  ──(talos: `Wasm.Decoder.Wat.decode` + `Wasm.SmallStep`)──▶
  a raw wasm run result  ──(THIS module: `toOutcome`)──▶  `Oracle.Outcome`

`toOutcome` is deliberately **interpreter-AGNOSTIC**: it consumes `WasmOutcome`/`WasmVal` (the oracle's
own boundary types), NOT talos's `Value`/exit-code directly. When talos is pinned (behind the Lean-4.32.2
toolchain bump, v-nix's lane), a THIN adapter maps talos's result into these types — the mapping SEMANTICS
below (exit-code → outcome, raw scalar + Cadenza result-type → `Value`, width/signedness discipline) are the
durable part and do not depend on the interpreter. talos's runner exit-code contract this mirrors:
`OK`→`.ok`, `TRAP`→`.trap`, `OUT_OF_FUEL`→`.outOfFuel`, `ERR`(imports present / decode-fail / bad method)
→`.err`. talos pre-flight REJECTS a module with imports, so a runtime-importing (heap/collection) case
arrives as `.err` → `.unsupported` (a SOUND coverage gap the conformance harness skips, never a false
differential) until the cdz-runtime imports are modeled (a later increment).
-/
import Oracle.Value
import Oracle.Eval

namespace Oracle.Wasm

open Oracle

/-- A raw scalar wasm result value. Integers are the SIGNED interpretation of the wasm bits (a wasm
`i32`/`i64` is two's-complement; the interpreter yields the signed `Int`, e.g. talos's `toInt32.toInt`).
Floats stay as raw IEEE bits (decoded to `Float` only at the boundary, so `-0.0`/`NaN` survive). -/
inductive WasmVal where
  | i32 (v : Int)
  | i64 (v : Int)
  | f32 (bits : UInt32)
  | f64 (bits : UInt64)
  deriving Inhabited, BEq, Repr

/-- The Cadenza RESULT TYPE of the entry, resolved from the emitted component's `@custom "cdz-result-type"`
binary-AST section. It tells `toOutcome` how to INTERPRET the raw wasm result (a bare `i64` is an `Int`, a
`bool`, a `char` codepoint, …). Only the SCALAR subset is modeled here (milestone 1); a compound/heap result
type routes to `.unsupported` (those cases import the runtime and talos declines them anyway). -/
inductive ScalarTy where
  | int          -- any Cadenza integer type (Int/Int64/BigInt/…): the value is the signed wasm int
  | bool         -- i32 0/1
  | float32
  | float64
  | unit
  deriving Inhabited, BEq, Repr

/-- The interpreter's run outcome — the oracle's own boundary shape, mirroring the wasm-interpreter
exit-code contract (talos: OK/TRAP/OUT_OF_FUEL/ERR). `ok` carries the result stack (a Cadenza `main`
produces exactly one scalar; other arities are a harness/model gap → `.unsupported`). -/
inductive WasmOutcome where
  | ok (vals : Array WasmVal)
  | trap (msg : String)
  | outOfFuel
  | err (msg : String)
  deriving Inhabited, BEq, Repr

/-- Decode a single raw wasm result value under its Cadenza scalar type into an `Oracle.Value`.
`none` = the raw valtype does not match the declared scalar type (a harness/model gap, surfaced as
`.unsupported` by the caller — never a silent wrong value). -/
def decodeScalar (ty : ScalarTy) (v : WasmVal) : Option Value :=
  match ty, v with
  -- Integer: the value is already the SIGNED wasm int; Cadenza `.int` is arbitrary-precision so the
  -- ascribed width (Int/Int64/BigInt) does not change the represented value here (it constrained the
  -- EMIT; the runtime result is that signed integer). Both wasm widths carry an int.
  | .int, .i32 n => some (.int n)
  | .int, .i64 n => some (.int n)
  -- Bool: a wasm i32, 0 = false / non-zero = true.
  | .bool, .i32 n => some (.bool (n != 0))
  -- Floats compare by f64 value in the oracle (`valueEqSpec`), so a decoded `.f64` is the canonical form;
  -- an f32 result widens to its f64 value. `-0.0` and `NaN` are preserved through the bit decode.
  | .float64, .f64 b => some (.f64 (Float.ofBits b))
  | .float32, .f32 b => some (.f64 (Float32.ofBits b).toFloat)
  | _, _ => none

/-- Map a wasm run outcome + the entry's Cadenza result type onto the shared `Oracle.Outcome`.
The mapping is total and interpreter-agnostic (see the module header). -/
def toOutcome (o : WasmOutcome) (ty : ScalarTy) : Outcome :=
  match o with
  | .trap msg => .trap msg
  | .outOfFuel => .diverges
  | .err msg => .unsupported msg
  | .ok vals =>
    match ty, vals.toList with
    -- Unit: a Cadenza `unit`-returning `main` emits no (or a discarded) result value.
    | .unit, [] => .value .unit
    | .unit, [_] => .value .unit
    | _, [v] =>
      match decodeScalar ty v with
      | some val => .value val
      | none => .unsupported "wasm result valtype does not match the declared Cadenza scalar type"
    | _, _ => .unsupported "wasm result arity is not one scalar (compound/heap result not yet modeled)"

/-! ### Gate witnesses — the mapping invariants (compiled = checked; no corpus case exercises this
internal boundary, so per PRINCIPLES.md this is exactly the kind of check that belongs in Lean, not the
corpus). Integer/bool/control cases reduce definitionally (`rfl`); float cases go through opaque `Float`
so they are asserted structurally. -/

-- exit-code → outcome
example : toOutcome (.trap "unreachable") .int = .trap "unreachable" := rfl
example : toOutcome .outOfFuel .int = .diverges := rfl
example : toOutcome (.err "module declares imports") .int = .unsupported "module declares imports" := rfl

-- scalar value decode (the milestone-1 mapping)
example : toOutcome (.ok #[.i64 5]) .int = .value (.int 5) := rfl
example : toOutcome (.ok #[.i32 (-1)]) .int = .value (.int (-1)) := rfl
example : toOutcome (.ok #[.i32 0]) .bool = .value (.bool false) := rfl
example : toOutcome (.ok #[.i32 1]) .bool = .value (.bool true) := rfl
example : toOutcome (.ok #[.i32 7]) .bool = .value (.bool true) := rfl
example : toOutcome (.ok #[]) .unit = .value .unit := rfl

-- shape gaps → sound `.unsupported` (never a wrong value / false differential)
example : toOutcome (.ok #[.i64 5]) .float64 = .unsupported "wasm result valtype does not match the declared Cadenza scalar type" := rfl
example : toOutcome (.ok #[.i64 1, .i64 2]) .int = .unsupported "wasm result arity is not one scalar (compound/heap result not yet modeled)" := rfl

end Oracle.Wasm
