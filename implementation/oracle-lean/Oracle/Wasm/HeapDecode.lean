/-
Structural decode of a heap-result handle into an `Oracle.Value` — the READ half of the heap-valued RESULT
decoder (the next big coverage lever: a heap-valued `main` returns an i32 HANDLE, and the final `HeapState`
lets us reconstruct its value instead of skipping the case).

This module is the pure, self-contained CORE: `handle + HeapState → raw Oracle.Value`. It produces the RAW
STRUCTURAL value — a byte buffer → `.bytes`, an array → `.tuple`, a sum DECLINES (needs the type's variant
names). The result-type FIXUPS (`.bytes`→`.str` / `.tuple`→`.record` guided by the entry's result type) and
`Eval.canonicalizeValue` (canonically sort/dedupe set/map/record to match Core's order-sensitive `valueEqSpec`)
are applied at the Outcome boundary by the driver/`toOutcome` wiring — a follow-up. Kept separate so this read
half (which only touches the modeled `HeapState` — its value read is witnessed) has no shared-seam surface.

Imports only `HeapHost` (the modeled heap) + `Oracle.Value` (the shared value domain) — both Std-only, so this
stays Mathlib-free like the rest of the execution path.
-/
import Oracle.Wasm.HeapHost
import Oracle.Value

namespace Oracle.Heap

open Oracle (Value)
open HeapState (isImmediate immIsInt immAsIntBits u64Signed immIsBool immAsBool immIsUnit)

/-- Structurally decode a heap handle + state → a raw `Oracle.Value` (PRE-canonicalize, PRE-type-fixup).
Fuel-bounded recursion. Immediates decode inline (fixnum → `.int`, atom → `.bool`/`.unit`); a live heap object
decodes by its `HeapValue`: scalars/bigint → `.int`, floats → `.f64`, a byte buffer → `.bytes` (a String
result is fixed up by result-type later), rational → `.rational`, array → `.tuple`, vec → `.list`, set → `.set`,
map → `.map` (all children decoded recursively, RAW order — canonicalized later). A SUM declines (`none` —
mapping a numeric disc → `Some/None/Ok/Err`/`variant name` needs the result type). `none` = undecodable
(sum / unknown / non-heap-non-immediate / exhausted fuel). -/
def decodeValueWork : Nat → HeapState → UInt32 → Option Value
  | 0,        _, _ => Option.none
  | fuel + 1, s, h =>
    if isImmediate h then
      if immIsInt h then Option.some (.int (u64Signed (immAsIntBits h)))
      else if immIsBool h then Option.some (.bool (immAsBool h))
      else if immIsUnit h then Option.some .unit
      else Option.none
    else match s.getObj? h with
      | Option.none   => Option.none
      | Option.some o =>
        if !o.live then Option.none
        else match o.value with
          | .int bits     => Option.some (.int (u64Signed bits))
          | .bigint v     => Option.some (.int v)
          | .bool b       => Option.some (.bool b)
          | .float bits   => Option.some (.f64 (Float.ofBits bits))
          | .float32 bits => Option.some (.f64 (Float32.ofBits bits).toFloat)
          | .bytes bs     => Option.some (.bytes (ByteArray.mk bs))
          | .rational nh dh =>
            match decodeValueWork fuel s nh, decodeValueWork fuel s dh with
            | Option.some (.int n), Option.some (.int d) => Option.some (.rational n d)
            | _, _ => Option.none
          | .array es => (es.toList.mapM (decodeValueWork fuel s)).map (fun vs => .tuple vs.toArray)
          | .vec es   => (es.toList.mapM (decodeValueWork fuel s)).map (fun vs => .list vs.toArray)
          | .set es   => (es.toList.mapM (decodeValueWork fuel s)).map (fun vs => .set vs.toArray)
          | .map es   =>
            ((List.range (es.size / 2)).mapM (fun i =>
                match decodeValueWork fuel s (es[2 * i]!), decodeValueWork fuel s (es[2 * i + 1]!) with
                | Option.some k, Option.some v => Option.some (k, v)
                | _, _ => Option.none)).map (fun ps => .map ps.toArray)
          | .sum _ _ => Option.none

/-- Decode a heap handle → `Oracle.Value` (fuel sized to the pool). The RAW structural value; the caller
applies result-type fixups + `canonicalizeValue`. -/
def HeapState.decodeValue? (s : HeapState) (h : UInt32) : Option Value :=
  decodeValueWork (s.objects.size + s.edges + 1) s h

/-! ### Witnesses — the structural decode round-trips the modeled heap values (compiled every build). -/

open HeapState

/-- A heap BigInt decodes to `Value.int` (arbitrary precision). -/
private def probeDecodeBigInt : Bool :=
  match bigintOfI64 ({} : HeapState) [.i64 42] with
  | .ret [.i32 h] s => s.decodeValue? h == Option.some (Value.int 42)
  | _              => false
example : probeDecodeBigInt = true := by native_decide

/-- An immediate fixnum decodes to `Value.int`; an immediate bool to `Value.bool`. -/
private def probeDecodeImm : Bool :=
  match boxInt ({} : HeapState) [.i64 7] with
  | .ret [.i32 hi] s0 =>
    match boxBool s0 [.i32 1] with
    | .ret [.i32 hb] s1 =>
      (s1.decodeValue? hi == Option.some (Value.int 7)) &&
      (s1.decodeValue? hb == Option.some (Value.bool true))
    | _ => false
  | _ => false
example : probeDecodeImm = true := by native_decide

/-- A 2-element array of (heap-float 1.0-bits, immediate int 5) decodes to a `Value.tuple`. -/
private def probeDecodeTuple : Bool :=
  match arrAlloc ({} : HeapState) [.i32 2] with
  | .ret [.i32 a] s0 =>
    match boxFloat s0 [.f64 (Float.toBits 1.5)] with
    | .ret [.i32 f] s1 =>
      match arrSet s1 [.i32 a, .i32 0, .i32 f] with
      | .ret [.i32 _] s2 =>
        match boxInt s2 [.i64 5] with
        | .ret [.i32 e] s3 =>
          match arrSet s3 [.i32 a, .i32 1, .i32 e] with
          | .ret [.i32 _] s4 =>
            s4.decodeValue? a == Option.some (Value.tuple #[Value.f64 1.5, Value.int 5])
          | _ => false
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeDecodeTuple = true := by native_decide

/-- A rational 1/2 (heap node of two BigInt leaves) decodes to `Value.rational 1 2`. -/
private def probeDecodeRational : Bool :=
  match bigintOfI64 ({} : HeapState) [.i64 1] with
  | .ret [.i32 n] s0 =>
    match bigintOfI64 s0 [.i64 2] with
    | .ret [.i32 d] s1 =>
      match rationalOf s1 [.i32 n, .i32 d] with
      | .ret [.i32 r] s2 => s2.decodeValue? r == Option.some (Value.rational 1 2)
      | _ => false
    | _ => false
  | _ => false
example : probeDecodeRational = true := by native_decide

/-- A list [10, 20] (immediate ints) decodes to `Value.list`. -/
private def probeDecodeList : Bool :=
  match vecEmpty ({} : HeapState) [] with
  | .ret [.i32 e0] s0 =>
    match boxInt s0 [.i64 10] with
    | .ret [.i32 x0] s1 =>
      match vecPush s1 [.i32 e0, .i32 x0] with
      | .ret [.i32 v1] s2 =>
        match boxInt s2 [.i64 20] with
        | .ret [.i32 x1] s3 =>
          match vecPush s3 [.i32 v1, .i32 x1] with
          | .ret [.i32 v2] s4 => s4.decodeValue? v2 == Option.some (Value.list #[Value.int 10, Value.int 20])
          | _ => false
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeDecodeList = true := by native_decide

end Oracle.Heap
