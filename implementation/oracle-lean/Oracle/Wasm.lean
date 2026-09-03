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
import Oracle.Ast
import Oracle.Value
import Oracle.Eval

namespace Oracle.Wasm

open Oracle
open Oracle.Ast

/-- A raw scalar wasm result value. Integers are the SIGNED interpretation of the wasm bits (a wasm
`i32`/`i64` is two's-complement; the interpreter yields the signed `Int`, e.g. talos's `toInt32.toInt`).
Floats stay as raw IEEE bits (decoded to `Float` only at the boundary, so `-0.0`/`NaN` survive). -/
inductive WasmVal where
  | i32 (v : Int)
  | i64 (v : Int)
  | f32 (bits : UInt32)
  | f64 (bits : UInt64)
  -- A heap-valued result already decoded to an `Oracle.Value` (the talos driver reads the final `HeapState`
  -- at the returned handle → structural value; see `Oracle.Wasm.HeapDecode`). Carries the compound directly so
  -- `toOutcomeHeap` can finalize it (Repr is dropped — `Value` has no `Repr`, and it was unused here).
  | compound (v : Value)
  deriving Inhabited, BEq

/-- The Cadenza RESULT TYPE of the entry, resolved from the emitted component's `@custom "cdz-result-type"`
binary-AST section. It tells `toOutcome` how to INTERPRET the raw wasm result (a bare `i64` is an `Int`, a
`bool`, a `char` codepoint, …). Only the SCALAR subset is modeled here (milestone 1); a compound/heap result
type routes to `.unsupported` (those cases import the runtime and talos declines them anyway). -/
inductive ScalarTy where
  | int          -- any SIGNED Cadenza integer type (Int/Int64/…): the value is the signed wasm int
  | uint         -- any UNSIGNED Cadenza integer type (UInt/UInt8/…/UInt64): the UNSIGNED reading of the bits
  | bool         -- i32 0/1
  | float32
  | float64
  | unit
  deriving Inhabited, BEq, DecidableEq, Repr

/-- The interpreter's run outcome — the oracle's own boundary shape, mirroring the wasm-interpreter
exit-code contract (talos: OK/TRAP/OUT_OF_FUEL/ERR). `ok` carries the result stack (a Cadenza `main`
produces exactly one scalar; other arities are a harness/model gap → `.unsupported`). -/
inductive WasmOutcome where
  | ok (vals : Array WasmVal) (leakCount : Nat := 0)  -- W6: `leakCount` = final `HeapState.liveCount` (0 = no leak / no-alloc / hostless); the `:= 0` default keeps every existing `.ok vs` construction valid.
  | trap (msg : String)
  | outOfFuel
  | err (msg : String)
  deriving Inhabited, BEq

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
  -- Unsigned: talos hands us the SIGNED reading (`talosToWasmVal` did `.toInt32.toInt`), so a large unsigned
  -- value arrives negative; recover the unsigned value by adding 2^width when negative. The `WasmVal` tag
  -- gives the width (i32 → +2^32, i64 → +2^64). A small unsigned (e.g. a UInt8 200) is already non-negative
  -- and passes through unchanged.
  | .uint, .i32 n => some (.int (if n < 0 then n + 4294967296 else n))            -- 2^32
  | .uint, .i64 n => some (.int (if n < 0 then n + 18446744073709551616 else n))  -- 2^64
  -- Bool: a wasm i32, 0 = false / non-zero = true.
  | .bool, .i32 n => some (.bool (n != 0))
  -- Floats compare by f64 value in the oracle (`valueEqSpec`), so a decoded `.f64` is the canonical form;
  -- an f32 result widens to its f64 value. `-0.0` and `NaN` are preserved through the bit decode.
  | .float64, .f64 b => some (.f64 (Float.ofBits b))
  | .float32, .f32 b => some (.f64 (Float32.ofBits b).toFloat)
  | _, _ => none

/-- Short name of a `ScalarTy` — for tagging the valtype-mismatch skip reason so the histogram shows which
`(ty, wasm valtype)` pairs `decodeScalar` doesn't handle (data-driven, like the result-type head tag). -/
def scalarTyName : ScalarTy → String
  | .int => "int" | .uint => "uint" | .bool => "bool"
  | .float32 => "float32" | .float64 => "float64" | .unit => "unit"

/-- Short name of a `WasmVal`'s wasm valtype. -/
def wasmValKind : WasmVal → String
  | .i32 _ => "i32" | .i64 _ => "i64" | .f32 _ => "f32" | .f64 _ => "f64" | .compound _ => "compound"

/-- Map a wasm run outcome + the entry's Cadenza result type onto the shared `Oracle.Outcome`.
The mapping is total and interpreter-agnostic (see the module header). -/
def toOutcome (o : WasmOutcome) (ty : ScalarTy) : Outcome :=
  match o with
  | .trap msg => .trap msg
  -- Out-of-fuel is INCONCLUSIVE, NOT a divergence: the interpreter's step budget was exhausted, but the
  -- program may terminate with more fuel (e.g. `06-numeric-model-1398` is a bounded 300k-iteration countdown
  -- ≈ 2.7M steps). Mapping it to `.diverges` produced a FALSE divergence against Core's value. A sound oracle
  -- SKIPs here (`.unsupported`) — never asserts non-termination it did not observe. Raising `talosDefaultFuel`
  -- lets bounded-long loops actually complete → AGREE; only a genuinely huge/infinite loop hits this skip.
  | .outOfFuel => .unsupported "wasm exceeded the interpreter fuel budget (inconclusive, not a divergence)"
  | .err msg => .unsupported msg
  | .ok vals _ =>          -- W6: leakCount does not affect the SCALAR value mapping (differential reads it separately)
    match ty, vals.toList with
    -- Unit: a Cadenza `unit`-returning `main` emits no (or a discarded) result value.
    | .unit, [] => .value .unit
    | .unit, [_] => .value .unit
    | _, [v] =>
      match decodeScalar ty v with
      | some val => .value val
      | none => .unsupported s!"wasm result valtype does not match the declared Cadenza scalar type (ty={scalarTyName ty}, wasm={wasmValKind v})"
    | _, _ => .unsupported "wasm result arity is not one scalar (compound/heap result not yet modeled)"

/-- Map a wasm run outcome for a HEAP result type onto `Oracle.Outcome`. A heap-valued `main` returns an i32
handle; the talos driver reads the final `HeapState` at that handle and hands back a decoded `.compound v`
(see `Oracle.Wasm.HeapDecode`), which this maps to `.value v`. (Canonicalization of set/map/record — to match
Core's order-sensitive `valueEqSpec` — is applied when those compounds are recognized; the current
`resultHeapDecodable?` heads either need none or are extended alongside it.) A non-`.compound` `.ok` → a
sound skip (the result was not a decodable heap object); trap/err/outOfFuel map as in `toOutcome`. -/
def toOutcomeHeap (o : WasmOutcome) : Outcome :=
  match o with
  | .trap msg  => .trap msg
  | .outOfFuel => .unsupported "wasm exceeded the interpreter fuel budget (inconclusive, not a divergence)"
  | .err msg   => .unsupported msg
  | .ok vals _ =>
    match vals.toList with
    | [.compound v] => .value v
    | _             => .unsupported "heap result was not a decodable heap value"

/-! ### Resolving the entry's result type from the emitted `cdz-result-type` section

The rcdzc wasm COMPONENT carries a `@custom "cdz-result-type"` section: a binary-AST module whose body is
a `(result-types (result-type <entry-name> <TypeName>) …)` — verified by emission (`Int`/`Bool`/`Float`
appear as the `<TypeName>` name leaf for scalar mains). The harness passes those raw section bytes here to
recover the `ScalarTy` that `toOutcome` needs. Decoding reuses the oracle's existing `Oracle.Ast` decoder
(a shared transport codec, not a semantic judgment — PRINCIPLES.md §1 nuance). -/

/-- The `name`-leaf bytes a node references, if it is an `atom → name` leaf. -/
def nameAtom? (m : Module) (i : Nat) : Option ByteArray :=
  match m.nodes[i]? with
  | some (.atom lid) =>
    match m.leaves[lid]? with
    | some (.name b) => some b
    | _ => none
  | _ => none

/-- The `name` OR `str` leaf bytes a node references. The emitted `cdz-result-type` spells the entry as a
`str` leaf (`(result-type "main" …)`), while heads/type names are `name` leaves — so matching the entry
needs both leaf kinds. -/
def atomText? (m : Module) (i : Nat) : Option ByteArray :=
  match m.nodes[i]? with
  | some (.atom lid) =>
    match m.leaves[lid]? with
    | some (.name b) => some b
    | some (.str b) => some b
    | _ => none
  | _ => none

/-- The scalar type's HEAD NAME at node `i`: a bare `name` atom's bytes, or — since `cdz-result-type` spells
a scalar type as a LIST `(Int 64)` / `(Float 64)` (head name + width), verified by emission — the list head
child's name. `none` for anything else. -/
def headTypeName? (m : Module) (i : Nat) : Option ByteArray :=
  match m.nodes[i]? with
  | some (.atom _) => nameAtom? m i
  | some (.list cs) => if cs.size ≥ 1 then nameAtom? m cs[0]! else none
  | _ => none

/-- Map an emitted Cadenza result-type HEAD NAME to a modeled `ScalarTy`. Only heads VERIFIED by emission
are mapped (`Int`/`Bool`/`Float`); anything else is `none` → the caller surfaces `.unsupported` (a sound
coverage gap, never a wrong value). The width child (`(Int 64)`) doesn't change the mapping — `decodeScalar`
maps any wasm int width → `.int`. Widen as more scalar heads are verified from real emits. -/
def scalarTyOfName? : ByteArray → Option ScalarTy := fun b =>
  if b == "Int".toUTF8 then some .int
  else if b == "UInt".toUTF8 then some .uint       -- all unsigned widths (UInt/UInt8/…/UInt64): unsigned read
  else if b == "Bool".toUTF8 then some .bool
  else if b == "Float".toUTF8 then some .float64   -- Cadenza `Float` is the f64 type
  else if b == "Unit".toUTF8 then some .unit
  else none

/-- Walk a decoded `cdz-result-type` module for `(result-type "<entry>" <Type>)` and map `<Type>`'s head
name to a `ScalarTy`. VERIFIED real structure: `(result-types (result-type "main" (Int 64)))` — the entry is
a `str` leaf, the type is a list `(Int 64)`. `none` if no such node / not a modeled scalar type.

EXACTLY-ONE-TYPE-CHILD (`cs.size == 3`): a scalar `main` has one result type child, so `(result-type "main"
<Ty>)` is 3 nodes. A TUPLE/multi-value `main` emits the flat form `(result-type "main" (Int 64) (Int 64) …)`
— multiple type children — which the old `≥ 3` guard silently truncated to `cs[2]` (the FIRST element),
leaking a compound result through as a scalar (the `06-numeric-model-1398` `(Tuple Int64 Int64 Int64 Int64)`
false-DIVERGE: Core ref was a tuple, wasm was decoded as one Int). Requiring `== 3` makes any multi-value
return `none` → a sound SKIP. (A `(Tuple …)`-wrapped single child is also rejected — `scalarTyOfName? "Tuple"`
is `none`.) -/
def resultScalarTyOfModule? (m : Module) (entry : ByteArray) : Option ScalarTy :=
  m.nodes.findSome? (fun node =>
    match node with
    | .list cs =>
      if cs.size == 3 && nameAtom? m cs[0]! == some "result-type".toUTF8
          && atomText? m cs[1]! == some entry then
        match headTypeName? m cs[2]! with
        | some ty => scalarTyOfName? ty
        | none => none
      else none
    | _ => none)

/-- Resolve the entry's Cadenza scalar result type from the raw `cdz-result-type` section bytes. -/
def resultScalarTy? (bytes : ByteArray) (entry : ByteArray) : Option ScalarTy :=
  match Ast.decode bytes with
  | .ok m => resultScalarTyOfModule? m entry
  | .error _ => none

/-- A single-child heap result-type HEAD the driver decodes structurally to the matching `Oracle.Value` with
NO result-type fixup: BigInt→`.int`, List→`.list`, Map→`.map`, Set→`.set`, Rational→`.rational`, Bytes→`.bytes`
(the decoder + `canonicalizeValue` produce exactly Core's form). String (→`.str`) and Record (→`.record`) are
EXCLUDED — they decode structurally to `.bytes`/`.tuple` and need a result-type fixup (a follow-up); so do
Nominal / Qty / sums. -/
def decodableHeapHead (ty : ByteArray) : Bool :=
  ty == "BigInt".toUTF8 || ty == "List".toUTF8 || ty == "Map".toUTF8
    || ty == "Set".toUTF8 || ty == "Rational".toUTF8 || ty == "Bytes".toUTF8

/-- Whether the entry's result type is a HEAP type the driver can decode + `toOutcomeHeap` finalizes WITHOUT a
result-type fixup. Recognizing it makes `runWasmWithLeak` INVOKE the driver (which structurally decodes the
heap-object result via `HeapState.decodeValue?`, then `canonicalizeValue`s it) instead of skipping. Covers the
single-head heap types (`decodableHeapHead`) AND the FLAT multi-value TUPLE form (`(result-type "main" T0 T1 …)`
→ `.tuple`). String/Record need fixups (later); Nominal/Qty/sums decline. -/
def resultHeapDecodableOfModule? (m : Module) (entry : ByteArray) : Bool :=
  m.nodes.any (fun node =>
    match node with
    | .list cs =>
      if nameAtom? m cs[0]! == some "result-type".toUTF8 && atomText? m cs[1]! == some entry then
        if cs.size == 3 then
          match headTypeName? m cs[2]! with
          | some ty => decodableHeapHead ty
          | none    => false
        else cs.size ≥ 4   -- ≥ 2 type children = the flat Tuple form → a `.tuple` result
      else false
    | _ => false)

/-- Whether the entry's result type is a driver-decodable heap type (from the raw section bytes). -/
def resultHeapDecodable? (bytes : ByteArray) (entry : ByteArray) : Bool :=
  match Ast.decode bytes with
  | .ok m => resultHeapDecodableOfModule? m entry
  | .error _ => false

/-- The result-type HEAD NAME of the entry when the node is PRESENT but its head is not a modeled scalar
type (`scalarTyOfName? = none`) — i.e. the exact head we skipped on, for diagnostics. `none` if the entry's
result-type node is absent entirely (a different skip cause) or its head IS modeled. Fuels v-lean-oracle's
skip-reason histogram: tagging the head into the skip reason surfaces which `ScalarTy` variants to add next. -/
def unmodeledResultHead? (m : Module) (entry : ByteArray) : Option ByteArray :=
  m.nodes.findSome? (fun node =>
    match node with
    | .list cs =>
      if cs.size == 3 && nameAtom? m cs[0]! == some "result-type".toUTF8
          && atomText? m cs[1]! == some entry then
        match headTypeName? m cs[2]! with
        | some ty => if (scalarTyOfName? ty).isNone then some ty else none
        | none => none
      else none
    | _ => none)

/-- The skip reason for an entry with no modeled scalar result type — tagged with the unresolved head name
when one is present (`… (head=Char)`), so the histogram shows head frequencies; generic otherwise. -/
def unmodeledResultReason (bytes : ByteArray) (entry : ByteArray) : String :=
  let base := "cdz-result-type: entry has no modeled scalar result type"
  match Ast.decode bytes with
  | .ok m =>
    match unmodeledResultHead? m entry with
    | some head => s!"{base} (head={(String.fromUTF8? head).getD "?"})"
    | none => base
  | .error _ => base

/-! ### The `run_wasm` composition spine + the interpreter seam

`runWasmWith` ties the two pure pieces together — `resultScalarTy?` (decode the entry's scalar type) and
`toOutcome` (map the raw result) — behind an injectable `Driver`, the exact seam the wasm interpreter
(talos) fills. Keeping the interpreter behind a parameter means (a) this composition + its invariants land
and gate on the current toolchain with no talos dependency, (b) the future talos slice is just "write the
adapter to `Driver`", and (c) v-lean-oracle's differential theorem can quantify over the driver (state it
for the talos driver specifically). The IMPURE step (component-unbundle + `wasm-tools print` → the `coreWat`
string, and reading the `resultTypeBytes` section) stays in the harness and feeds this pure function. -/

/-- A trial to run against an emitted program: the entry export + its arguments (milestone-1 mains are
nullary, so `args` is empty; host responses arrive in a later increment). -/
structure Trial where
  entry : String
  args : Array WasmVal := #[]
  deriving Inhabited

/-- The interpreter SEAM: run a core-module WAT's entry for a trial, yielding a `WasmOutcome`. talos plugs
in here (an adapter over `Wasm.Decoder.Wat` + `Wasm.SmallStep`) once the Lean-4.32.2 toolchain lands. -/
abbrev Driver := (coreWat : String) → (trial : Trial) → WasmOutcome

/-- The heap-leak census on a wasm run's `.ok` outcome (W6): 0 for trap/err/outOfFuel or a no-alloc run. -/
def wasmLeakOf : WasmOutcome → Nat
  | .ok _ lc => lc
  | _ => 0

/-- Like `runWasmWith` but ALSO surfaces the final heap-leak census (`HeapState.liveCount`, W6): the pair
`(scalar Outcome, leakCount)`. Drives ONCE. The differential asserts `leakCount == 0` on a value-agreeing
run (the Perceus dynamic witness); `leakCount` is 0 whenever the run did not `.ok` or allocated nothing. -/
def runWasmWithLeak (drive : Driver) (coreWat : String) (resultTypeBytes : ByteArray)
    (trial : Trial) : Outcome × Nat :=
  match resultScalarTy? resultTypeBytes trial.entry.toUTF8 with
  | some ty => let o := drive coreWat trial; (toOutcome o ty, wasmLeakOf o)
  | none =>
    -- Not a scalar result type: if it is a driver-decodable HEAP result type, run + decode the returned
    -- handle (the ~heap-valued-result lever); else a sound skip.
    if resultHeapDecodable? resultTypeBytes trial.entry.toUTF8 then
      let o := drive coreWat trial; (toOutcomeHeap o, wasmLeakOf o)
    else (.unsupported (unmodeledResultReason resultTypeBytes trial.entry.toUTF8), 0)

/-- The pure `run_wasm` boundary: resolve the entry's scalar result type, drive the interpreter on the
core-module WAT, and map the result to an `Oracle.Outcome` — the leak-agnostic projection of
`runWasmWithLeak` (a `none` result type → `.unsupported`, driver never invoked for an unmodeled shape). -/
def runWasmWith (drive : Driver) (coreWat : String) (resultTypeBytes : ByteArray)
    (trial : Trial) : Outcome :=
  (runWasmWithLeak drive coreWat resultTypeBytes trial).1

/-! ### Gate witnesses — the mapping invariants (compiled = checked; no corpus case exercises this
internal boundary, so per PRINCIPLES.md this is exactly the kind of check that belongs in Lean, not the
corpus). Integer/bool/control cases reduce definitionally (`rfl`); float cases go through opaque `Float`
so they are asserted structurally. -/

-- exit-code → outcome
example : toOutcome (.trap "unreachable") .int = .trap "unreachable" := rfl
example : toOutcome .outOfFuel .int = .unsupported "wasm exceeded the interpreter fuel budget (inconclusive, not a divergence)" := rfl
example : toOutcome (.err "module declares imports") .int = .unsupported "module declares imports" := rfl

-- scalar value decode (the milestone-1 mapping)
example : toOutcome (.ok #[.i64 5]) .int = .value (.int 5) := rfl
example : toOutcome (.ok #[.i32 (-1)]) .int = .value (.int (-1)) := rfl
example : toOutcome (.ok #[.i32 0]) .bool = .value (.bool false) := rfl
example : toOutcome (.ok #[.i32 1]) .bool = .value (.bool true) := rfl
example : toOutcome (.ok #[.i32 7]) .bool = .value (.bool true) := rfl
example : toOutcome (.ok #[]) .unit = .value .unit := rfl
-- unsigned decode: small unsigned passes through; a large unsigned arrives signed-negative and is recovered
-- by +2^width (i32→+2^32, i64→+2^64). (`Outcome` has BEq not DecidableEq, so assert via `==`.)
example : (toOutcome (.ok #[.i32 200]) .uint == .value (.int 200)) = true := by native_decide
example : (toOutcome (.ok #[.i32 (-1)]) .uint == .value (.int 4294967295)) = true := by native_decide
example : (toOutcome (.ok #[.i64 (-1)]) .uint == .value (.int 18446744073709551615)) = true := by native_decide

-- shape gaps → sound `.unsupported` (never a wrong value / false differential)
example : toOutcome (.ok #[.i64 5]) .float64 = .unsupported "wasm result valtype does not match the declared Cadenza scalar type (ty=float64, wasm=i64)" := rfl
example : toOutcome (.ok #[.i64 1, .i64 2]) .int = .unsupported "wasm result arity is not one scalar (compound/heap result not yet modeled)" := rfl

-- result-type name → ScalarTy (the verified scalar spellings; others decline to none)
example : scalarTyOfName? "Int".toUTF8 = some .int := by native_decide
example : scalarTyOfName? "UInt".toUTF8 = some .uint := by native_decide
example : scalarTyOfName? "Bool".toUTF8 = some .bool := by native_decide
example : scalarTyOfName? "Float".toUTF8 = some .float64 := by native_decide
example : scalarTyOfName? "String".toUTF8 = none := by native_decide

-- walking a `(result-type main <Ty>)` module recovers the entry's scalar type
example :
    resultScalarTyOfModule?
      { leaves := #[.name "result-type".toUTF8, .name "main".toUTF8, .name "Int".toUTF8],
        nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2]], root := 3 }
      "main".toUTF8 = some .int := by native_decide
example :
    resultScalarTyOfModule?
      { leaves := #[.name "result-type".toUTF8, .name "main".toUTF8, .name "Bool".toUTF8],
        nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2]], root := 3 }
      "main".toUTF8 = some .bool := by native_decide
-- an entry that is not present → none
example :
    resultScalarTyOfModule?
      { leaves := #[.name "result-type".toUTF8, .name "main".toUTF8, .name "Int".toUTF8],
        nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2]], root := 3 }
      "other".toUTF8 = none := by native_decide
-- the REAL emitted structure `(result-types (result-type "main" (Int 64)))`: entry is a `str` leaf, the type
-- is a list `(Int 64)`. (Verified against a real corpus case's cdz-result-type section; the earlier flat
-- `(result-type main Int)` hand-form was too simplistic and made every corpus case skip — this pins reality.)
example :
    resultScalarTyOfModule?
      { leaves := #[.name "result-types".toUTF8, .name "result-type".toUTF8, .str "main".toUTF8,
                    .name "Int".toUTF8, .intLit false .dec (ByteArray.mk #[64])],
        nodes := #[.atom 1, .atom 2, .atom 3, .atom 4, .list #[2, 3], .list #[0, 1, 4], .atom 0,
                   .list #[6, 5]], root := 7 }
      "main".toUTF8 = some .int := by native_decide
-- a TUPLE/multi-value main emits the FLAT form `(result-type "main" (Int 64) (Int 64))` — MULTIPLE type
-- children (`cs.size == 4`), which must be REJECTED (→ none → sound SKIP), never truncated to the first
-- element. Pins the `06-numeric-model-1398` `(Tuple Int64 …)` false-DIVERGE fix (`cs.size == 3` guard).
example :
    resultScalarTyOfModule?
      { leaves := #[.name "result-types".toUTF8, .name "result-type".toUTF8, .str "main".toUTF8,
                    .name "Int".toUTF8, .intLit false .dec (ByteArray.mk #[64])],
        nodes := #[.atom 1, .atom 2, .atom 3, .atom 4, .list #[2, 3], .list #[2, 3], .list #[0, 1, 4, 5],
                   .atom 0, .list #[7, 6]], root := 8 }
      "main".toUTF8 = none := by native_decide

/-- The `cdz-result-type` section bytes for `(result-type main <tyName>)` — a real encode via `Oracle.Ast`,
so the end-to-end `runWasmWith` witnesses exercise the actual section round-trip (encode → decode → resolve). -/
private def rtBytes (tyName : String) : ByteArray :=
  Ast.encode
    { leaves := #[.name "result-type".toUTF8, .name "main".toUTF8, .name tyName.toUTF8],
      nodes := #[.atom 0, .atom 1, .atom 2, .list #[0, 1, 2]], root := 3 }

-- end-to-end `run_wasm` spine, through a STUB driver + a real result-type section round-trip.
-- (`Outcome` derives `BEq` but not `DecidableEq`, so assert equality via `==`.)
example : (runWasmWith (fun _ _ => .ok #[.i64 5]) "(module)" (rtBytes "Int") { entry := "main" }
    == .value (.int 5)) = true := by native_decide
example : (runWasmWith (fun _ _ => .ok #[.i32 1]) "(module)" (rtBytes "Bool") { entry := "main" }
    == .value (.bool true)) = true := by native_decide
-- interpreter trap propagates (result type resolved but the driver traps)
example : (runWasmWith (fun _ _ => .trap "unreachable") "(module)" (rtBytes "Int") { entry := "main" }
    == .trap "unreachable") = true := by native_decide
-- an unmodeled result-type spelling short-circuits to `.unsupported` (driver never consulted)
example : (runWasmWith (fun _ _ => .ok #[.i64 5]) "(module)" (rtBytes "Widget") { entry := "main" }
    == .unsupported "cdz-result-type: entry has no modeled scalar result type (head=Widget)") = true := by native_decide

-- A HEAP result type ("BigInt") routes to the heap path: `resultHeapDecodable?` recognizes it → the driver is
-- INVOKED (not skipped), and `toOutcomeHeap` finalizes the driver's decoded `.compound` to `.value`. (Here a
-- stub driver returns the `.compound`; the driver's actual HeapState decode is witnessed in `Oracle.Wasm.Talos`.)
example : (runWasmWith (fun _ _ => .ok #[.compound (.int 1000000000)]) "(module)" (rtBytes "BigInt") { entry := "main" }
    == .value (.int 1000000000)) = true := by native_decide
-- A still-unmodeled heap head (Nominal/Qty/…) is NOT heap-decodable → the driver is NOT invoked → sound skip.
example : (runWasmWith (fun _ _ => .ok #[.i64 0]) "(module)" (rtBytes "Widget") { entry := "main" }
    == .unsupported "cdz-result-type: entry has no modeled scalar result type (head=Widget)") = true := by native_decide
-- The extended heap heads route to the heap path too: a List / Set / Map / Rational / Bytes result-type →
-- driver invoked → the decoded `.compound` → `.value`.
example : (runWasmWith (fun _ _ => .ok #[.compound (.list #[.int 1, .int 2])]) "(module)" (rtBytes "List") { entry := "main" }
    == .value (.list #[.int 1, .int 2])) = true := by native_decide
example : (runWasmWith (fun _ _ => .ok #[.compound (.set #[.int 1, .int 2])]) "(module)" (rtBytes "Set") { entry := "main" }
    == .value (.set #[.int 1, .int 2])) = true := by native_decide
-- String / Record are NOT heap-decodable yet (need a fixup) → sound skip (driver not invoked).
example : (runWasmWith (fun _ _ => .ok #[.i64 0]) "(module)" (rtBytes "String") { entry := "main" }
    == .unsupported "cdz-result-type: entry has no modeled scalar result type (head=String)") = true := by native_decide

end Oracle.Wasm
