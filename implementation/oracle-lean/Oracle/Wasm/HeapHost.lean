/-
A clean-room Lean host-function model of the cdz-runtime `"heap"` import surface — the W5.1a + W5.1b slices.

The emitted core module imports the value-heap runtime ops (module name `"heap"`, kebab-case op names)
that talos declines today because it runs with an empty host. This file models those ops as pure Lean
state transformers over a `HeapState`, so `talosDriver` can supply them as `HostFn`s (W5.1c) and the
heap/collection corpus cases become runnable instead of skipped.

Modeled subset so far:
  * refcount / liveness core: `dup`, `drop`, `live-objects`   (W5.1a)
  * boxing:                   box-{int,float,float32,bool} + get-{int,float,float32,bool}   (W5.1a)
  * arrays (fixed-arity tuple/record product): arr-{alloc,set,get,len}   (W5.1b)
  * immortality:              mark-immortal, mark-immortal-deep   (W5.1b)
The Perceus REUSE ops (`reset`, arr-alloc-reuse, sum-new-reuse) are runtime behavior underspecified by the
WIT signature; they land in W5.4 after syncing the exact reuse-token contract with the runtime owner. A
module importing a not-yet-modeled op is simply not covered → skips soundly (W5.1c's cover-check gates it).

INDEPENDENCE (the whole point of the differential): the semantics are modeled from the SPEC's observable
value form + refcount discipline (cdz-runtime/wit/runtime.wit), NOT by linking the real cdz-runtime wasm —
else a runtime bug would hide on both sides. The state is refcount + liveness aware from the start (the
operator's Perceus constraint): an access to a freed handle traps (use-after-free); a `drop` of a freed /
rc-0 handle traps (double-free); `drop` at rc 0 recursively drops the freed node's owned child handles
(the cascade); `mark-immortal` makes dup/drop no-ops + excludes the node from the census; `live-objects`
counts non-immortal live objects, so a leak is a non-zero count at end of run.

Imports ONLY talos's `Syntax` (`Value`/`ValueType`/`Store`) + `Host` (`HostFn`/`HostResult`) — both are
Std-only, so this stays Mathlib-free like the rest of the execution path.
-/
import Interpreter.Wasm.Syntax
import Interpreter.Wasm.Host

open _root_.Wasm (Value ValueType Store HostFn HostResult)

namespace Oracle.Heap

/-- A runtime heap value (the observable value form, NOT the Rust byte layout). Boxed scalars store the
exact wasm payload they were boxed from so `get` round-trips `box` bit-for-bit (`bool` normalizes: any
non-zero i32 → `true`). An `array` is the fixed-arity positional product (tuple/record): its slots are
child handles (`0` = a null / unset slot); the array OWNS one reference per non-null slot, so freeing it
cascades a `drop` into each. A `map` is the positional key→value collection (the literal-construction form;
the functional HAMT `insert`/`lookup`/… is a later slice): its slots are the INTERLEAVED
`[k0,v0,k1,v1,…]` handles (2·len slots), all owned like an array's. -/
inductive HeapValue where
  | int     (bits : UInt64)
  | float   (bits : UInt64)
  | float32 (bits : UInt32)
  | bool    (b : Bool)
  | array   (elems : Array UInt32)
  | map     (entries : Array UInt32)
  | set     (elems : Array UInt32)
  | vec     (elems : Array UInt32)
  | bytes   (bs : Array UInt8)
  | sum     (disc : UInt32) (payload : UInt32)
  | bigint  (v : Int)
  | rational (num : UInt32) (den : UInt32)
deriving Repr, DecidableEq, Inhabited, BEq

/-- The number of owned child handles a value carries (0 for a scalar; the slot count for an array/map/set/
vec) — the child set the free-cascade and `mark-immortal-deep` walk. -/
def HeapValue.arity : HeapValue → Nat
  | .array e | .map e | .set e | .vec e => e.size
  | .sum _ _                            => 1
  | .rational _ _                       => 2
  | _                                   => 0

/-- The owned child handles (array/map/set/vec slots; `[]` for a scalar). -/
def HeapValue.children : HeapValue → List UInt32
  | .array e | .map e | .set e | .vec e => e.toList
  | .sum _ p                            => [p]
  | .rational n d                       => [n, d]
  | _                                   => []

/-- One heap object: its value, refcount, liveness (`false` once freed at rc 0), and immortality flag
(immortal objects have a sentinel rc — `dup`/`drop` are no-ops — and are excluded from the leak census). -/
structure HeapObject where
  value    : HeapValue
  rc       : Nat
  live     : Bool
  immortal : Bool := false
deriving Repr, Inhabited

/-- The host state `α`: a growable pool of heap objects addressed by a **1-based** `u32` handle (object at
0-based index `i` ↔ handle `i + 1`; handle `0` is the reserved NULL sentinel — an unset array slot — so it
never names a real object). Fresh allocations append, so a handle is stable for the run (a freed slot is
marked, never reused in W5.1a/b — the reuse specialization is W5.4). -/
structure HeapState where
  objects : Array HeapObject := #[]
deriving Repr, Inhabited

/-- The outcome of a heap op: either result values + the new state, or a trap (a UAF / double-free / OOB /
bad argument). Mapped to talos's `HostResult` by `toHostFn`. -/
inductive HeapResult where
  | ret  (vals : List Value) (s : HeapState)
  | trap (msg : String)
deriving Repr

namespace HeapState

/-- The leak oracle: the number of live, non-immortal objects. Must be 0 at end of run, else a leak. -/
def liveCount (s : HeapState) : Nat :=
  s.objects.foldl (fun n o => if o.live && !o.immortal then n + 1 else n) 0

/-- Total owned child references across the pool — an upper bound on the cascade/mark-deep work, used to
size the traversal fuel. -/
def edges (s : HeapState) : Nat :=
  s.objects.foldl (fun n o => n + o.value.arity) 0

/-! ### Immediate (tagged-inline) handles — the runtime's handle ABI (cdz-runtime lib.rs:735-835). A handle's
low 2 bits tag it: `00` = HEAP pointer / NULL; `01` = fixnum INT (value = arith `h >> 2`, window ±2^29);
`10` = ATOM (`0b0010` = UNIT, bool = `0b0110`/`0b10110`). `box-int(fixnum)`/`box-bool`/`arr-alloc(0)` produce
immediates (NO heap object, NO refcount); `dup`/`drop` are NO-OPS on them and they are EXCLUDED from the leak
census. This matters because the COMPILER pushes `imm_unit` (0b0010) as an `i32.const` constant (from the
`cdz-abi` `IMM_UNIT` section), so my host RECEIVES immediate handles it must decode — an ABI fact, not a
clean-room choice. Heap handles are therefore encoded `(index+1) <<< 2` (low 2 bits `00`, never the `0` NULL). -/

/-- A handle is immediate iff its low 2 bits are non-zero (NULL = 0 is heap-tagged, not immediate). -/
def isImmediate (h : UInt32) : Bool := (h &&& 3) != 0

/-- The inline `unit` (empty tuple / `arr-alloc(0)` / nullary-sum payload): atom subkind `00`, bits `0b0010`. -/
def immUnit : UInt32 := 2

/-- An inline boolean: false = `0b0110`, true = `0b10110` (value in bit 4). -/
def immBool (b : Bool) : UInt32 := if b then 22 else 6

/-- The inline fixnum window `[-2^29, 2^29 - 1]`. -/
def fixnumFits (v : Int) : Bool := (-536870912 : Int) ≤ v && v ≤ (536870911 : Int)

/-- Encode a fixnum (given its i64 bits) as an immediate: `(low-32-bits <<< 2) ||| 0b01` (matches the runtime
`imm_int`; caller has checked `fixnumFits`). -/
def immInt (bits : UInt64) : UInt32 := (bits.toUInt32 <<< 2) ||| 1

/-- The signed i32 value of a 32-bit word (for decoding an immediate's payload). -/
def u32Signed (h : UInt32) : Int :=
  let n : Int := (h.toNat : Int)
  if n ≥ 2147483648 then n - 4294967296 else n

/-- Two's-complement i64 bits of an `Int` (to reconstruct a `get-int` result). -/
def intToU64Bits (v : Int) : UInt64 :=
  (if v ≥ 0 then v else v + 18446744073709551616).toNat.toUInt64

/-- Decode a fixnum immediate to its i64 bits: arithmetic `>> 2` (= floor-div by 4) of the signed word. -/
def immAsIntBits (h : UInt32) : UInt64 := intToU64Bits (Int.fdiv (u32Signed h) 4)

/-- Decode an inline boolean (bit 4). -/
def immAsBool (h : UInt32) : Bool := ((h >>> 4) &&& 1) != 0

/-- Whether an immediate is the fixnum-int kind (tag `01`). -/
def immIsInt (h : UInt32) : Bool := (h &&& 3) == 1
/-- Whether an immediate is a bool atom (`10`, subkind `01`). -/
def immIsBool (h : UInt32) : Bool := (h &&& 3) == 2 && ((h >>> 2) &&& 3) == 1
/-- Whether an immediate is the unit atom (`10`, subkind `00`). -/
def immIsUnit (h : UInt32) : Bool := (h &&& 3) == 2 && ((h >>> 2) &&& 3) == 0

/-- Allocate a fresh live HEAP object (rc 1); return its handle + the new state. Heap handles are
`(index+1) <<< 2` (low 2 bits `00`, distinct from `0` NULL and from immediates). -/
def alloc (s : HeapState) (v : HeapValue) : UInt32 × HeapState :=
  (((s.objects.size + 1) <<< 2).toUInt32,
   { s with objects := s.objects.push { value := v, rc := 1, live := true } })

/-- Look up a HEAP object by handle (`none` for NULL, an immediate, or a handle never allocated). -/
def getObj? (s : HeapState) (h : UInt32) : Option HeapObject :=
  if h == 0 || isImmediate h then none else s.objects[(h >>> 2).toNat - 1]?

/-- Overwrite the heap object at `h` (caller has checked `h` is a live heap handle via `getObj?`). -/
def setObj (s : HeapState) (h : UInt32) (o : HeapObject) : HeapState :=
  { s with objects := s.objects.set! ((h >>> 2).toNat - 1) o }

/-! ### Boxing -/

/-- Box a scalar: allocate + return the handle as an i32. -/
def box (s : HeapState) (v : HeapValue) : HeapResult :=
  let (h, s') := s.alloc v
  .ret [.i32 h] s'

/-- Signed `Int` value of a 64-bit word (for the fixnum-window check). -/
def u64Signed (n : UInt64) : Int :=
  let x : Int := (n.toNat : Int)
  if x ≥ 9223372036854775808 then x - 18446744073709551616 else x

/-- `box-int(v)`: a fixnum-window value inlines as an `imm_int` IMMEDIATE (no heap, no rc — matching the
runtime); a larger value heap-boxes. -/
def boxInt : HeapState → List Value → HeapResult
  | s, [.i64 n] => if fixnumFits (u64Signed n) then .ret [.i32 (immInt n)] s else s.box (.int n)
  | s, _        => .trap "box-int: expected (i64)"

def boxFloat : HeapState → List Value → HeapResult
  | s, [.f64 b] => s.box (.float b)
  | s, _        => .trap "box-float: expected (f64)"

def boxFloat32 : HeapState → List Value → HeapResult
  | s, [.f32 b] => s.box (.float32 b)
  | s, _        => .trap "box-float32: expected (f32)"

/-- `box-bool(v)`: always inlines as an `imm_bool` IMMEDIATE (no heap, no rc — matching the runtime). -/
def boxBool : HeapState → List Value → HeapResult
  | s, [.i32 n] => .ret [.i32 (immBool (n != 0))] s
  | s, _        => .trap "box-bool: expected (i32)"

/-- Read a boxed handle, requiring it to be live (else UAF) and of the expected shape. -/
def getWith (s : HeapState) (h : UInt32) (who : String) (f : HeapValue → Option Value) : HeapResult :=
  match s.getObj? h with
  | none   => .trap s!"{who}: unknown handle {h}"
  | some o =>
    if !o.live then .trap s!"{who}: use-after-free (handle {h} freed)"
    else match f o.value with
      | some v => .ret [v] s
      | none   => .trap s!"{who}: handle {h} holds the wrong boxed type"

def getInt : HeapState → List Value → HeapResult
  | s, [.i32 h] =>
    if isImmediate h then
      if immIsInt h then .ret [.i64 (immAsIntBits h)] s
      else .trap s!"get-int: immediate handle {h} is not an int"
    else s.getWith h "get-int" (fun | .int bits => some (.i64 bits) | _ => none)
  | s, _ => .trap "get-int: expected (i32)"

def getFloat : HeapState → List Value → HeapResult
  | s, [.i32 h] => s.getWith h "get-float" (fun | .float bits => some (.f64 bits) | _ => none)
  | s, _        => .trap "get-float: expected (i32)"

def getFloat32 : HeapState → List Value → HeapResult
  | s, [.i32 h] => s.getWith h "get-float32" (fun | .float32 bits => some (.f32 bits) | _ => none)
  | s, _        => .trap "get-float32: expected (i32)"

def getBool : HeapState → List Value → HeapResult
  | s, [.i32 h] =>
    if isImmediate h then
      if immIsBool h then .ret [.i32 (if immAsBool h then 1 else 0)] s
      else .trap s!"get-bool: immediate handle {h} is not a bool"
    else s.getWith h "get-bool" (fun | .bool b => some (.i32 (if b then 1 else 0)) | _ => none)
  | s, _ => .trap "get-bool: expected (i32)"

/-! ### Arrays (the fixed-arity tuple/record product) -/

/-- `arr-alloc(len) → handle`: a fresh array of `len` NULL (`0`) handle-slots. -/
def arrAlloc : HeapState → List Value → HeapResult
  | s, [.i32 len] =>
    -- A length-0 array is the inline UNIT immediate (matches the runtime + the compiler's pushed `imm_unit`
    -- for empty tuples / nullary-sum payloads); a non-empty array heap-allocates its slots.
    if len == 0 then .ret [.i32 immUnit] s
    else s.box (.array (List.replicate len.toNat (0 : UInt32)).toArray)
  | s, _ => .trap "arr-alloc: expected (i32)"

/-- `arr-set(arr, i, elem) → arr`: store `elem` at slot `i` WITHOUT dup (an ownership MOVE — the array
takes `elem`'s existing reference), returning the array handle for threading. OOB traps. -/
def arrSet : HeapState → List Value → HeapResult
  | s, [.i32 arr, .i32 i, .i32 elem] =>
    if isImmediate arr then .trap s!"arr-set: index {i} out of bounds (inline unit has 0 slots)"
    else match s.getObj? arr with
    | none   => .trap s!"arr-set: unknown handle {arr}"
    | some o =>
      if !o.live then .trap s!"arr-set: use-after-free (handle {arr} freed)"
      else match o.value with
        | .array elems =>
          if i.toNat < elems.size then
            .ret [.i32 arr] (s.setObj arr { o with value := .array (elems.set! i.toNat elem) })
          else .trap s!"arr-set: index {i} out of bounds (len {elems.size})"
        | _ => .trap s!"arr-set: handle {arr} is not an array"
  | s, _ => .trap "arr-set: expected (i32, i32, i32)"

/-- `arr-get(arr, i) → elem`: the slot handle, BORROWED (rc unchanged; the array keeps ownership — a caller
that keeps the handle gets a compiler-emitted `dup`). OOB traps. -/
def arrGet : HeapState → List Value → HeapResult
  | s, [.i32 arr, .i32 i] =>
    if isImmediate arr then .trap s!"arr-get: index {i} out of bounds (inline unit has 0 slots)"
    else match s.getObj? arr with
    | none   => .trap s!"arr-get: unknown handle {arr}"
    | some o =>
      if !o.live then .trap s!"arr-get: use-after-free (handle {arr} freed)"
      else match o.value with
        | .array elems =>
          match elems[i.toNat]? with
          | some slot => .ret [.i32 slot] s
          | none      => .trap s!"arr-get: index {i} out of bounds (len {elems.size})"
        | _ => .trap s!"arr-get: handle {arr} is not an array"
  | s, _ => .trap "arr-get: expected (i32, i32)"

/-- `arr-len(arr) → len`: the slot count. -/
def arrLen : HeapState → List Value → HeapResult
  | s, [.i32 arr] =>
    if isImmediate arr then .ret [.i32 0] s   -- the inline unit is a length-0 array
    else match s.getObj? arr with
    | none   => .trap s!"arr-len: unknown handle {arr}"
    | some o =>
      if !o.live then .trap s!"arr-len: use-after-free (handle {arr} freed)"
      else match o.value with
        | .array elems => .ret [.i32 elems.size.toUInt32] s
        | _            => .trap s!"arr-len: handle {arr} is not an array"
  | s, _ => .trap "arr-len: expected (i32)"

/-! ### Maps — the positional literal-construction form (a map node = interleaved `[k0,v0,k1,v1,…]` slots).
Keys AND values are owned handles, stored VERBATIM (no sort/dedup — ordering is compiler-owned language
semantics). The functional HAMT form (`map-empty`/`insert`/`lookup`/`remove`/`merge`/`iter`) is a later
slice and needs value-equality key matching + canonical order. -/

/-- `map-alloc(len) → m`: a fresh map of `len` NULL `(key,value)` pairs (2·len null slots). -/
def mapAlloc : HeapState → List Value → HeapResult
  | s, [.i32 len] => s.box (.map (List.replicate (2 * len.toNat) (0 : UInt32)).toArray)
  | s, _          => .trap "map-alloc: expected (i32)"

/-- `map-set(m, i, key, value) → m`: store the pair at index `i` WITHOUT dup (ownership MOVE of both key
and value), returning the map handle for threading. OOB traps. -/
def mapSet : HeapState → List Value → HeapResult
  | s, [.i32 m, .i32 i, .i32 key, .i32 value] =>
    match s.getObj? m with
    | none   => .trap s!"map-set: unknown handle {m}"
    | some o =>
      if !o.live then .trap s!"map-set: use-after-free (handle {m} freed)"
      else match o.value with
        | .map entries =>
          if 2 * i.toNat + 1 < entries.size then
            .ret [.i32 m] (s.setObj m
              { o with value := .map ((entries.set! (2 * i.toNat) key).set! (2 * i.toNat + 1) value) })
          else .trap s!"map-set: index {i} out of bounds (len {entries.size / 2})"
        | _ => .trap s!"map-set: handle {m} is not a map"
  | s, _ => .trap "map-set: expected (i32, i32, i32, i32)"

/-- `map-key(m, i) → key`: the key handle at pair `i`, BORROWED. OOB traps. -/
def mapKey : HeapState → List Value → HeapResult
  | s, [.i32 m, .i32 i] =>
    match s.getObj? m with
    | none   => .trap s!"map-key: unknown handle {m}"
    | some o =>
      if !o.live then .trap s!"map-key: use-after-free (handle {m} freed)"
      else match o.value with
        | .map entries =>
          match entries[2 * i.toNat]? with
          | some k => .ret [.i32 k] s
          | none   => .trap s!"map-key: index {i} out of bounds (len {entries.size / 2})"
        | _ => .trap s!"map-key: handle {m} is not a map"
  | s, _ => .trap "map-key: expected (i32, i32)"

/-- `map-val(m, i) → value`: the value handle at pair `i`, BORROWED. OOB traps. -/
def mapVal : HeapState → List Value → HeapResult
  | s, [.i32 m, .i32 i] =>
    match s.getObj? m with
    | none   => .trap s!"map-val: unknown handle {m}"
    | some o =>
      if !o.live then .trap s!"map-val: use-after-free (handle {m} freed)"
      else match o.value with
        | .map entries =>
          match entries[2 * i.toNat + 1]? with
          | some v => .ret [.i32 v] s
          | none   => .trap s!"map-val: index {i} out of bounds (len {entries.size / 2})"
        | _ => .trap s!"map-val: handle {m} is not a map"
  | s, _ => .trap "map-val: expected (i32, i32)"

/-- `map-len(m) → len`: the pair count (= slots / 2). -/
def mapLen : HeapState → List Value → HeapResult
  | s, [.i32 m] =>
    match s.getObj? m with
    | none   => .trap s!"map-len: unknown handle {m}"
    | some o =>
      if !o.live then .trap s!"map-len: use-after-free (handle {m} freed)"
      else match o.value with
        | .map entries => .ret [.i32 (entries.size / 2).toUInt32] s
        | _            => .trap s!"map-len: handle {m} is not a map"
  | s, _ => .trap "map-len: expected (i32)"

/-! ### Functional map — READ-ONLY / non-consuming ops (`map-empty`/`map-lookup`/`map-size`, W5.2b-1). Key
matching is structural VALUE-equality (champ_eq), NOT handle identity. The CONSUMING ops (`map-insert`/
`map-remove`/`map-merge`) with their dup-and-drop ownership transfer are W5.2b-2; iteration/`to-list`
(canonical order, co-owned with v-lean-oracle) is W5.2c. -/

/-- Structural VALUE-equality over a worklist of handle pairs — the key/elem match for map/set ops. Same
shape + equal scalar payloads, recursing into array/map children positionally (matches champ_eq's structural
walk); handle identity short-circuits. Fuel-bounded tail recursion on a decreasing `Nat` (the proven
`dropCascade` pattern); the caller sizes fuel ≥ the compared structures' total handles. -/
def valueEqWork : Nat → HeapState → List (UInt32 × UInt32) → Bool
  | 0,        _, _        => false
  | _+1,      _, []       => true
  | fuel + 1, s, (h1, h2) :: rest =>
    if h1 == h2 then valueEqWork fuel s rest
    else match s.getObj? h1, s.getObj? h2 with
      | some o1, some o2 =>
        match o1.value, o2.value with
        | .int a,     .int b     => a == b && valueEqWork fuel s rest
        | .float a,   .float b   => a == b && valueEqWork fuel s rest
        | .float32 a, .float32 b => a == b && valueEqWork fuel s rest
        | .bool a,    .bool b     => a == b && valueEqWork fuel s rest
        | .bytes a,   .bytes b   => a == b && valueEqWork fuel s rest
        | .array e1,  .array e2  =>
          e1.size == e2.size && valueEqWork fuel s (e1.toList.zip e2.toList ++ rest)
        | .map e1,    .map e2    =>
          e1.size == e2.size && valueEqWork fuel s (e1.toList.zip e2.toList ++ rest)
        | .sum d1 p1, .sum d2 p2 =>
          d1 == d2 && valueEqWork fuel s ((p1, p2) :: rest)
        | .bigint a,  .bigint b  => a == b && valueEqWork fuel s rest
        | .rational n1 d1, .rational n2 d2 =>
          valueEqWork fuel s ((n1, n2) :: (d1, d2) :: rest)
        | _,          _          => false
      | _, _ => false

/-- Structural value-equality of two handles (fuel sized to the pool). -/
def valueEq (s : HeapState) (h1 h2 : UInt32) : Bool :=
  s.valueEqWork (s.objects.size + s.edges + 1) [(h1, h2)]

/-- `map-empty() → m`: a fresh empty map. -/
def mapEmpty : HeapState → List Value → HeapResult
  | s, []     => s.box (.map #[])
  | _, _ :: _ => .trap "map-empty: expected ()"

/-- `map-lookup(m, k) → val | 0`: the value for the first key structurally-equal to `k`, else NULL (`0`).
BORROWS m + k (rc unchanged). -/
def mapLookup : HeapState → List Value → HeapResult
  | s, [.i32 m, .i32 k] =>
    match s.getObj? m with
    | none   => .trap s!"map-lookup: unknown handle {m}"
    | some o =>
      if !o.live then .trap s!"map-lookup: use-after-free (handle {m} freed)"
      else match o.value with
        | .map entries =>
          match (List.range (entries.size / 2)).find? (fun i => s.valueEq (entries[2 * i]!) k) with
          | some i => .ret [.i32 (entries[2 * i + 1]!)] s
          | none   => .ret [.i32 0] s
        | _ => .trap s!"map-lookup: handle {m} is not a map"
  | s, _ => .trap "map-lookup: expected (i32, i32)"

/-- `map-size(m) → count`: the entry count (= slots / 2), O(1). -/
def mapSize : HeapState → List Value → HeapResult
  | s, [.i32 m] =>
    match s.getObj? m with
    | none   => .trap s!"map-size: unknown handle {m}"
    | some o =>
      if !o.live then .trap s!"map-size: use-after-free (handle {m} freed)"
      else match o.value with
        | .map entries => .ret [.i32 (entries.size / 2).toUInt32] s
        | _            => .trap s!"map-size: handle {m} is not a map"
  | s, _ => .trap "map-size: expected (i32)"

/-! ### Refcount / liveness core (+ the free-cascade) -/

/-- `dup(h)`: an IMMEDIATE handle is a NO-OP (immediates carry no rc); require live (else UAF); on an
immortal node it is a NO-OP (sentinel rc); else rc++. -/
def dup : HeapState → List Value → HeapResult
  | s, [.i32 h] =>
    if isImmediate h then .ret [] s
    else match s.getObj? h with
    | none   => .trap s!"dup: unknown handle {h}"
    | some o =>
      if o.immortal then .ret [] s
      else if !o.live then .trap s!"dup: use-after-free (handle {h} freed)"
      else .ret [] (s.setObj h { o with rc := o.rc + 1 })
  | s, _ => .trap "dup: expected (i32)"

/-- The free-cascade: decrement each handle on the worklist; when one reaches rc 0, free it and enqueue its
owned children. Fuel-bounded on a strictly-decreasing `Nat` (structural termination); the caller sizes fuel
≥ the total work so it never truncates for a well-formed acyclic heap. Internal skips (null / absent /
immortal / already-freed) keep the cascade total — the observable double-free TRAP is raised by the
top-level `drop` op, not here. -/
def dropCascade : Nat → HeapState → List UInt32 → HeapState
  | 0,        s, _        => s
  | _+1,      s, []       => s
  | fuel + 1, s, h :: rest =>
    if h == 0 then dropCascade fuel s rest
    else match s.getObj? h with
      | none   => dropCascade fuel s rest
      | some o =>
        if o.immortal || !o.live || o.rc == 0 then dropCascade fuel s rest
        else
          let rc' := o.rc - 1
          let s1  := s.setObj h { o with rc := rc', live := rc' != 0 }
          if rc' == 0 then dropCascade fuel s1 (o.value.children ++ rest)
          else dropCascade fuel s1 rest

/-- `drop(h)`: require live + rc>0 + non-immortal-or-no-op (else double-free); rc--; at 0 → freed +
recursively drop the owned children (the cascade). An IMMEDIATE handle is a NO-OP (no rc, nothing to free).
No result. -/
def drop : HeapState → List Value → HeapResult
  | s, [.i32 h] =>
    if isImmediate h then .ret [] s
    else match s.getObj? h with
    | none   => .trap s!"drop: unknown handle {h}"
    | some o =>
      if o.immortal then .ret [] s
      else if !o.live then .trap s!"drop: double-free (handle {h} already freed)"
      else if o.rc == 0 then .trap s!"drop: double-free (handle {h} refcount already zero)"
      else .ret [] (s.dropCascade (s.objects.size + s.edges + 1) [h])
  | s, _ => .trap "drop: expected (i32)"

/-! ### Immortality (build-once statics) -/

/-- `mark-immortal(h) → h`: convert a node to IMMORTAL (dup/drop become no-ops + excluded from the census).
Returns the same handle. -/
def markImmortal : HeapState → List Value → HeapResult
  | s, [.i32 h] =>
    if isImmediate h then .ret [.i32 h] s   -- an immediate is already census-excluded; return unchanged
    else match s.getObj? h with
    | none   => .trap s!"mark-immortal: unknown handle {h}"
    | some o => .ret [.i32 h] (s.setObj h { o with immortal := true })
  | s, _ => .trap "mark-immortal: expected (i32)"

/-- The transitive mark: mark each handle on the worklist immortal and enqueue its children. Idempotent +
DAG-safe (an already-immortal node is skipped — no re-walk, no double count), fuel-bounded like the
cascade. -/
def markDeep : Nat → HeapState → List UInt32 → HeapState
  | 0,        s, _        => s
  | _+1,      s, []       => s
  | fuel + 1, s, h :: rest =>
    if h == 0 then markDeep fuel s rest
    else match s.getObj? h with
      | none   => markDeep fuel s rest
      | some o =>
        if o.immortal then markDeep fuel s rest
        else markDeep fuel (s.setObj h { o with immortal := true }) (o.value.children ++ rest)

/-- `mark-immortal-deep(h) → h`: mark the root AND every transitively-reachable node immortal. -/
def markImmortalDeep : HeapState → List Value → HeapResult
  | s, [.i32 h] =>
    if isImmediate h then .ret [.i32 h] s
    else match s.getObj? h with
    | none   => .trap s!"mark-immortal-deep: unknown handle {h}"
    | some _ => .ret [.i32 h] (s.markDeep (s.objects.size + s.edges + 1) [h])
  | s, _ => .trap "mark-immortal-deep: expected (i32)"

/-- `live-objects()`: the live non-immortal census (the leak oracle), as an i32. NOTE: the compiler never
emits a call to this (it is a post-run verification aid), so no corpus module imports it — the leak
assertion is a post-run inspection of `liveCount` (W6). Kept here for completeness / a direct test. -/
def liveObjects : HeapState → List Value → HeapResult
  | s, []     => .ret [.i32 s.liveCount.toUInt32] s
  | _, _ :: _ => .trap "live-objects: expected ()"

/-! ### Functional map — CONSUMING ops (`map-insert`/`map-remove`, W5.2b-2). Ownership transfer by
DUP-AND-DROP (v-runtime's champ.rs contract): a consumed map's KEPT children are dup'd into the fresh result,
then the map is dropped. On a UNIQUE map the drop's cascade cancels the dups (entries transfer, spine freed)
AND frees the handles that LEAVE (they weren't dup'd); on a SHARED map (rc>1) the drop does not cascade, so
the map survives with its own refs and the result holds the dup'd copies — both correct, uniformly. The
old-value-on-replace / removed key+value are handled by NOT dup'ing them (never an explicit drop — that would
wrongly free a shared map's still-owned entry). Only the redundant incoming KEY on a replace is explicitly
dropped (it is the caller's consumed arg, never stored). `map-merge` is W5.2b-3. -/

/-- rc++ on a handle (immortal = no-op) — transfers a kept child into a fresh result map. -/
def dupH (s : HeapState) (h : UInt32) : HeapState :=
  match s.getObj? h with
  | some o => if o.immortal then s else s.setObj h { o with rc := o.rc + 1 }
  | none   => s

/-- Drop a single handle with the standard cascade (fuel sized to the pool). -/
def dropH (s : HeapState) (h : UInt32) : HeapState :=
  s.dropCascade (s.objects.size + s.edges + 1) [h]

/-- `map-insert(m, k, v) → m'` [consumes m, k, v]: key↦val, last-write-wins by structural value-eq. EXISTING
key → keep the stored key, take v, the old value is freed by the consumed map's cascade + the redundant
incoming k is dropped. NEW key → append (k, v). -/
def mapInsert : HeapState → List Value → HeapResult
  | s, [.i32 m, .i32 k, .i32 v] =>
    match s.getObj? m with
    | none   => .trap s!"map-insert: unknown handle {m}"
    | some o =>
      if !o.live then .trap s!"map-insert: use-after-free (handle {m} freed)"
      else match o.value with
        | .map entries =>
          match (List.range (entries.size / 2)).find? (fun i => s.valueEq (entries[2 * i]!) k) with
          | some i =>
            let result := entries.set! (2 * i + 1) v
            let keep := (List.range entries.size).filterMap
              (fun j => if j == 2 * i + 1 then none else some entries[j]!)
            let s1 := keep.foldl (fun acc h => acc.dupH h) s
            let (r, s2) := s1.alloc (.map result)
            .ret [.i32 r] ((s2.dropH m).dropH k)
          | none =>
            let result := (entries.push k).push v
            let s1 := entries.toList.foldl (fun acc h => acc.dupH h) s
            let (r, s2) := s1.alloc (.map result)
            .ret [.i32 r] (s2.dropH m)
        | _ => .trap s!"map-insert: handle {m} is not a map"
  | s, _ => .trap "map-insert: expected (i32, i32, i32)"

/-- `map-remove(m, k) → m'` [consumes m, BORROWS k]: m without the entry whose key value-equals k; the
removed key+value are freed by the consumed map's cascade (not kept). An ABSENT key is a NO-OP (identity:
m returned unchanged, no alloc, no drop). -/
def mapRemove : HeapState → List Value → HeapResult
  | s, [.i32 m, .i32 k] =>
    match s.getObj? m with
    | none   => .trap s!"map-remove: unknown handle {m}"
    | some o =>
      if !o.live then .trap s!"map-remove: use-after-free (handle {m} freed)"
      else match o.value with
        | .map entries =>
          match (List.range (entries.size / 2)).find? (fun i => s.valueEq (entries[2 * i]!) k) with
          | none   => .ret [.i32 m] s
          | some i =>
            let keep := (List.range entries.size).filterMap
              (fun j => if j == 2 * i || j == 2 * i + 1 then none else some entries[j]!)
            let s1 := keep.foldl (fun acc h => acc.dupH h) s
            let (r, s2) := s1.alloc (.map keep.toArray)
            .ret [.i32 r] (s2.dropH m)
        | _ => .trap s!"map-remove: handle {m} is not a map"
  | s, _ => .trap "map-remove: expected (i32, i32)"

/-- `map-merge(a, b) → a∪b` [consumes both, b WINS on conflict]: fold b's entries into `a` (per v-runtime's
champ.rs: `op_dup(k); op_dup(v); acc := op_map_insert(acc,k,v); … op_drop(b)`). Each of b's pairs is dup'd so
b keeps its refs during the fold, then inserted (b-wins conflict handled by `mapInsert`'s replace: a's losing
value + the redundant b-key are dropped, b's value taken); finally b is dropped. -/
def mapMerge : HeapState → List Value → HeapResult
  | s, [.i32 a, .i32 b] =>
    match s.getObj? a, s.getObj? b with
    | none, _ => .trap s!"map-merge: unknown handle {a}"
    | _, none => .trap s!"map-merge: unknown handle {b}"
    | some oa, some ob =>
      if !oa.live then .trap s!"map-merge: use-after-free (handle {a} freed)"
      else if !ob.live then .trap s!"map-merge: use-after-free (handle {b} freed)"
      else match oa.value, ob.value with
        | .map _, .map bEntries =>
          let (accH, s') := (List.range (bEntries.size / 2)).foldl
            (fun (acc : UInt32 × HeapState) i =>
              let (curAcc, st) := acc
              let bk := bEntries[2 * i]!
              let bv := bEntries[2 * i + 1]!
              let st1 := (st.dupH bk).dupH bv
              match mapInsert st1 [.i32 curAcc, .i32 bk, .i32 bv] with
              | .ret [.i32 acc'] st2 => (acc', st2)
              | _                    => (curAcc, st1))
            (a, s)
          .ret [.i32 accH] (s'.dropH b)
        | _, _ => .trap s!"map-merge: handle {a} or {b} is not a map"
  | s, _ => .trap "map-merge: expected (i32, i32)"

/-! ### Set — core ops (W5.2d-1). A set mirrors a map with stride 1 (each entry is ONE element handle, no
value column) over the same value-eq + dup-and-drop machinery. `set-union`/`-intersection`/`-difference` are
W5.2d-2; `set-iter`/`-to-list` (value-sorted, W5.2c order) are W5.2c. `set-empty`/`-contains`/`-size` are
read-only; `set-insert`/`-remove` consume (per v-runtime's stride-1 champ.rs contract). -/

/-- `set-empty() → s`: a fresh empty set. -/
def setEmpty : HeapState → List Value → HeapResult
  | s, []     => s.box (.set #[])
  | _, _ :: _ => .trap "set-empty: expected ()"

/-- `set-contains(s, elem) → bool` (i32 0/1): membership by structural value-eq. BORROWS s + elem. -/
def setContains : HeapState → List Value → HeapResult
  | s, [.i32 st, .i32 elem] =>
    match s.getObj? st with
    | none   => .trap s!"set-contains: unknown handle {st}"
    | some o =>
      if !o.live then .trap s!"set-contains: use-after-free (handle {st} freed)"
      else match o.value with
        | .set elems =>
          let present := (elems.toList).any (fun e => s.valueEq e elem)
          .ret [.i32 (if present then 1 else 0)] s
        | _ => .trap s!"set-contains: handle {st} is not a set"
  | s, _ => .trap "set-contains: expected (i32, i32)"

/-- `set-size(s) → count`: the element count. -/
def setSize : HeapState → List Value → HeapResult
  | s, [.i32 st] =>
    match s.getObj? st with
    | none   => .trap s!"set-size: unknown handle {st}"
    | some o =>
      if !o.live then .trap s!"set-size: use-after-free (handle {st} freed)"
      else match o.value with
        | .set elems => .ret [.i32 elems.size.toUInt32] s
        | _          => .trap s!"set-size: handle {st} is not a set"
  | s, _ => .trap "set-size: expected (i32)"

/-- `set-insert(s, elem) → s'` [consumes s, elem]: add elem unless already present (by value-eq), in which
case the incoming DUPLICATE elem is dropped (the set keeps its stored element). Dup-and-drop transfer. -/
def setInsert : HeapState → List Value → HeapResult
  | s, [.i32 st, .i32 elem] =>
    match s.getObj? st with
    | none   => .trap s!"set-insert: unknown handle {st}"
    | some o =>
      if !o.live then .trap s!"set-insert: use-after-free (handle {st} freed)"
      else match o.value with
        | .set elems =>
          if (elems.toList).any (fun e => s.valueEq e elem) then
            -- already present: keep all elements (dup'd), drop the incoming duplicate
            let s1 := elems.toList.foldl (fun acc h => acc.dupH h) s
            let (r, s2) := s1.alloc (.set elems)
            .ret [.i32 r] ((s2.dropH st).dropH elem)
          else
            let s1 := elems.toList.foldl (fun acc h => acc.dupH h) s
            let (r, s2) := s1.alloc (.set (elems.push elem))
            .ret [.i32 r] (s2.dropH st)
        | _ => .trap s!"set-insert: handle {st} is not a set"
  | s, _ => .trap "set-insert: expected (i32, i32)"

/-- `set-remove(s, elem) → s'` [consumes s, BORROWS elem]: s without the element value-equal to elem (the
removed stored element is freed by the consumed set's cascade). ABSENT elem = no-op identity. -/
def setRemove : HeapState → List Value → HeapResult
  | s, [.i32 st, .i32 elem] =>
    match s.getObj? st with
    | none   => .trap s!"set-remove: unknown handle {st}"
    | some o =>
      if !o.live then .trap s!"set-remove: use-after-free (handle {st} freed)"
      else match o.value with
        | .set elems =>
          match (List.range elems.size).find? (fun i => s.valueEq (elems[i]!) elem) with
          | none   => .ret [.i32 st] s
          | some i =>
            let keep := (List.range elems.size).filterMap
              (fun j => if j == i then none else some elems[j]!)
            let s1 := keep.foldl (fun acc h => acc.dupH h) s
            let (r, s2) := s1.alloc (.set keep.toArray)
            .ret [.i32 r] (s2.dropH st)
        | _ => .trap s!"set-remove: handle {st} is not a set"
  | s, _ => .trap "set-remove: expected (i32, i32)"

/-! ### Set — the 2-set ops (W5.2d-2, per v-runtime's champ.rs contract, each CONSUMES both). union =
insert-all-of-b (dup drops an incoming duplicate); intersection keeps a's elems that are also in b (a's
others + all of b freed); difference (a\b) keeps a's elems NOT in b (a's others + all of b freed). -/

/-- `set-union(a, b) → a∪b` [consumes both]: fold b's elements into a via `setInsert` (dedup drops incoming
duplicates), then drop b — the same shape as `map-merge`. -/
def setUnion : HeapState → List Value → HeapResult
  | s, [.i32 a, .i32 b] =>
    match s.getObj? a, s.getObj? b with
    | none, _ => .trap s!"set-union: unknown handle {a}"
    | _, none => .trap s!"set-union: unknown handle {b}"
    | some oa, some ob =>
      if !oa.live then .trap s!"set-union: use-after-free (handle {a} freed)"
      else if !ob.live then .trap s!"set-union: use-after-free (handle {b} freed)"
      else match oa.value, ob.value with
        | .set _, .set bElems =>
          let (accH, s') := (List.range bElems.size).foldl
            (fun (acc : UInt32 × HeapState) i =>
              let (cur, st) := acc
              let be := bElems[i]!
              let st1 := st.dupH be
              match setInsert st1 [.i32 cur, .i32 be] with
              | .ret [.i32 acc'] st2 => (acc', st2)
              | _                    => (cur, st1))
            (a, s)
          .ret [.i32 accH] (s'.dropH b)
        | _, _ => .trap s!"set-union: handle {a} or {b} is not a set"
  | s, _ => .trap "set-union: expected (i32, i32)"

/-- `set-intersection(a, b) → {x ∈ a | x ∈ b}` [consumes both]: keep a's elements value-equal to some b
element; a's other elements are freed by a's cascade, all of b is freed. -/
def setIntersection : HeapState → List Value → HeapResult
  | s, [.i32 a, .i32 b] =>
    match s.getObj? a, s.getObj? b with
    | none, _ => .trap s!"set-intersection: unknown handle {a}"
    | _, none => .trap s!"set-intersection: unknown handle {b}"
    | some oa, some ob =>
      if !oa.live then .trap s!"set-intersection: use-after-free (handle {a} freed)"
      else if !ob.live then .trap s!"set-intersection: use-after-free (handle {b} freed)"
      else match oa.value, ob.value with
        | .set aElems, .set bElems =>
          let keep := aElems.toList.filter (fun ae => bElems.toList.any (fun be => s.valueEq ae be))
          let s1 := keep.foldl (fun acc h => acc.dupH h) s
          let (r, s2) := s1.alloc (.set keep.toArray)
          .ret [.i32 r] ((s2.dropH a).dropH b)
        | _, _ => .trap s!"set-intersection: handle {a} or {b} is not a set"
  | s, _ => .trap "set-intersection: expected (i32, i32)"

/-- `set-difference(a, b) → {x ∈ a | x ∉ b}` [consumes both]: keep a's elements NOT value-equal to any b
element; a's elements that are in b are freed by a's cascade, all of b is freed. -/
def setDifference : HeapState → List Value → HeapResult
  | s, [.i32 a, .i32 b] =>
    match s.getObj? a, s.getObj? b with
    | none, _ => .trap s!"set-difference: unknown handle {a}"
    | _, none => .trap s!"set-difference: unknown handle {b}"
    | some oa, some ob =>
      if !oa.live then .trap s!"set-difference: use-after-free (handle {a} freed)"
      else if !ob.live then .trap s!"set-difference: use-after-free (handle {b} freed)"
      else match oa.value, ob.value with
        | .set aElems, .set bElems =>
          let keep := aElems.toList.filter (fun ae => !bElems.toList.any (fun be => s.valueEq ae be))
          let s1 := keep.foldl (fun acc h => acc.dupH h) s
          let (r, s2) := s1.alloc (.set keep.toArray)
          .ret [.i32 r] ((s2.dropH a).dropH b)
        | _, _ => .trap s!"set-difference: handle {a} or {b} is not a set"
  | s, _ => .trap "set-difference: expected (i32, i32)"

/-! ### List (`vec-*`, W5-vec-1) — the language's growable LIST (a persistent sequence; the runtime uses a
radix trie, unobservable, so a flat element array is a faithful value model). Per value-heap-runtime.md
"Constructors Consume And Accessors Borrow": `vec-empty` produces a new owned list; `vec-get` BORROWS
(rc unchanged, OOB traps); `vec-push`/`vec-update` are CONSTRUCTORS that CONSUME the list + element and
produce a new owned list (dup-and-drop transfer, same as map-insert — dupH/dropH no-op on immediate elements).
vec-concat, vec-prepend, vec-of-arr are a later slice. -/

/-- `vec-empty() → v`: a fresh empty list. -/
def vecEmpty : HeapState → List Value → HeapResult
  | s, []     => s.box (.vec #[])
  | _, _ :: _ => .trap "vec-empty: expected ()"

/-- `vec-len(v) → len`: the element count. -/
def vecLen : HeapState → List Value → HeapResult
  | s, [.i32 v] =>
    match s.getObj? v with
    | none   => .trap s!"vec-len: unknown handle {v}"
    | some o =>
      if !o.live then .trap s!"vec-len: use-after-free (handle {v} freed)"
      else match o.value with
        | .vec elems => .ret [.i32 elems.size.toUInt32] s
        | _          => .trap s!"vec-len: handle {v} is not a list"
  | s, _ => .trap "vec-len: expected (i32)"

/-- `vec-get(v, i) → elem`: the element handle at index `i`, BORROWED. OOB traps. -/
def vecGet : HeapState → List Value → HeapResult
  | s, [.i32 v, .i32 i] =>
    match s.getObj? v with
    | none   => .trap s!"vec-get: unknown handle {v}"
    | some o =>
      if !o.live then .trap s!"vec-get: use-after-free (handle {v} freed)"
      else match o.value with
        | .vec elems =>
          match elems[i.toNat]? with
          | some e => .ret [.i32 e] s
          | none   => .trap s!"vec-get: index {i} out of bounds (len {elems.size})"
        | _ => .trap s!"vec-get: handle {v} is not a list"
  | s, _ => .trap "vec-get: expected (i32, i32)"

/-- `vec-push(v, elem) → v'` [consumes v, elem]: a new list = v's elements with `elem` appended. Dup-and-drop
transfer of the kept elements; `elem` moved in. -/
def vecPush : HeapState → List Value → HeapResult
  | s, [.i32 v, .i32 elem] =>
    match s.getObj? v with
    | none   => .trap s!"vec-push: unknown handle {v}"
    | some o =>
      if !o.live then .trap s!"vec-push: use-after-free (handle {v} freed)"
      else match o.value with
        | .vec elems =>
          let s1 := elems.toList.foldl (fun acc h => acc.dupH h) s
          let (r, s2) := s1.alloc (.vec (elems.push elem))
          .ret [.i32 r] (s2.dropH v)
        | _ => .trap s!"vec-push: handle {v} is not a list"
  | s, _ => .trap "vec-push: expected (i32, i32)"

/-- `vec-update(v, i, elem) → v'` [consumes v, elem]: a new list = v with index `i` set to `elem`; the old
element at `i` is freed by the consumed list's cascade (not kept). OOB traps. -/
def vecUpdate : HeapState → List Value → HeapResult
  | s, [.i32 v, .i32 i, .i32 elem] =>
    match s.getObj? v with
    | none   => .trap s!"vec-update: unknown handle {v}"
    | some o =>
      if !o.live then .trap s!"vec-update: use-after-free (handle {v} freed)"
      else match o.value with
        | .vec elems =>
          if i.toNat < elems.size then
            let keep := (List.range elems.size).filterMap
              (fun j => if j == i.toNat then none else some elems[j]!)
            let s1 := keep.foldl (fun acc h => acc.dupH h) s
            let (r, s2) := s1.alloc (.vec (elems.set! i.toNat elem))
            .ret [.i32 r] (s2.dropH v)
          else .trap s!"vec-update: index {i} out of bounds (len {elems.size})"
        | _ => .trap s!"vec-update: handle {v} is not a list"
  | s, _ => .trap "vec-update: expected (i32, i32, i32)"

/-! ### List — the extra constructors (`vec-concat`/`-prepend`/`-of-arr`/`-drop`, indices 55/97/60/72). The
runtime uses an RRB relaxed-radix trie (O(log N) concat/split), unobservable — a flat element array is a
faithful value model. All CONSUME per "Constructors Consume And Accessors Borrow" (dup the KEPT element
handles into the fresh list, then drop the consumed list/arr — on a UNIQUE input the drop's cascade cancels
the dups and frees the handles that LEAVE; the moved-in element is not dup'd), matching `vec-push`. -/

/-- `vec-concat(a, b) → v` [CONSUMES a, b]: a's elements then b's. -/
def vecConcat : HeapState → List Value → HeapResult
  | s, [.i32 a, .i32 b] =>
    match s.getObj? a, s.getObj? b with
    | none, _ => .trap s!"vec-concat: unknown handle {a}"
    | _, none => .trap s!"vec-concat: unknown handle {b}"
    | some oa, some ob =>
      if !oa.live then .trap s!"vec-concat: use-after-free (handle {a} freed)"
      else if !ob.live then .trap s!"vec-concat: use-after-free (handle {b} freed)"
      else match oa.value, ob.value with
        | .vec ea, .vec eb =>
          let joined := ea ++ eb
          let s1 := joined.toList.foldl (fun acc h => acc.dupH h) s
          let (r, s2) := s1.alloc (.vec joined)
          .ret [.i32 r] ((s2.dropH a).dropH b)
        | _, _ => .trap s!"vec-concat: handle {a} or {b} is not a list"
  | s, _ => .trap "vec-concat: expected (i32, i32)"

/-- `vec-prepend(v, elem) → v'` [CONSUMES v, elem]: `elem` then v's elements (front-growth twin of push). -/
def vecPrepend : HeapState → List Value → HeapResult
  | s, [.i32 v, .i32 elem] =>
    match s.getObj? v with
    | none   => .trap s!"vec-prepend: unknown handle {v}"
    | some o =>
      if !o.live then .trap s!"vec-prepend: use-after-free (handle {v} freed)"
      else match o.value with
        | .vec elems =>
          let s1 := elems.toList.foldl (fun acc h => acc.dupH h) s
          let (r, s2) := s1.alloc (.vec (#[elem] ++ elems))
          .ret [.i32 r] (s2.dropH v)
        | _ => .trap s!"vec-prepend: handle {v} is not a list"
  | s, _ => .trap "vec-prepend: expected (i32, i32)"

/-- `vec-of-arr(arr) → v` [CONSUMES arr]: a list of the array's element handles (order preserved). -/
def vecOfArr : HeapState → List Value → HeapResult
  | s, [.i32 arr] =>
    if isImmediate arr then s.box (.vec #[])   -- the inline unit is the empty array → the empty list
    else match s.getObj? arr with
    | none   => .trap s!"vec-of-arr: unknown handle {arr}"
    | some o =>
      if !o.live then .trap s!"vec-of-arr: use-after-free (handle {arr} freed)"
      else match o.value with
        | .array elems =>
          let s1 := elems.toList.foldl (fun acc h => acc.dupH h) s
          let (r, s2) := s1.alloc (.vec elems)
          .ret [.i32 r] (s2.dropH arr)
        | _ => .trap s!"vec-of-arr: handle {arr} is not an array"
  | s, _ => .trap "vec-of-arr: expected (i32)"

/-- `vec-drop(v, index) → v'` [CONSUMES v]: the TAIL `[index, len)`, dropping the prefix `[0, index)`; the
dropped prefix elements leave (freed by the consumed list's cascade). `index ≥ len` → the empty list. -/
def vecDrop : HeapState → List Value → HeapResult
  | s, [.i32 v, .i32 index] =>
    match s.getObj? v with
    | none   => .trap s!"vec-drop: unknown handle {v}"
    | some o =>
      if !o.live then .trap s!"vec-drop: use-after-free (handle {v} freed)"
      else match o.value with
        | .vec elems =>
          let keep := (elems.toList.drop index.toNat).toArray
          let s1 := keep.toList.foldl (fun acc h => acc.dupH h) s
          let (r, s2) := s1.alloc (.vec keep)
          .ret [.i32 r] (s2.dropH v)
        | _ => .trap s!"vec-drop: handle {v} is not a list"
  | s, _ => .trap "vec-drop: expected (i32, i32)"

/-! ### Map/Set → List (`map-to-list`/`set-to-list`, W5.2c) — the program-observable enumeration, and the
ONLY one: the language has no fold/iter/keys, and the raw CHAMP cursor's hash order is NEVER program-observable
(v-runtime, from champ.rs/prelude). Both BORROW their collection (and the shape `desc`, which we ignore — for
a scalar key/elem the order IS the value order) and return a fresh owned `List`. `set-to-list` → `List a` of
the elements in canonical VALUE order; `map-to-list` → `List (Tuple k v)` where each entry is a fresh
2-element array `[key, value]` in canonical KEY order, each component dup'd from the map (co-owned alongside
it, matching value_codec.rs `op_map_to_list`). Canonical order = v-lean-oracle's `cmpValue` (Int signed,
Bool false<true; String/Bytes lexicographic arrive with W5.3). An unorderable (non-scalar) key never reaches
here — the compiler rejects it upstream (CDZ0203) — so we order scalar keys and sort any non-scalar last. -/

/-- The canonical sort key of a scalar handle: `(rank, payload)` lexicographic — Bool (rank 0, false<true) <
Int (rank 1, signed value); a non-scalar sorts last (rank 2). Immediate-aware. -/
def scalarOrdKey (s : HeapState) (h : UInt32) : Nat × Int :=
  if isImmediate h then
    if immIsBool h then (0, if immAsBool h then 1 else 0)
    else if immIsInt h then (1, u64Signed (immAsIntBits h))
    else (2, 0)
  else match s.getObj? h with
    | some o => match o.value with
      | .bool b   => (0, if b then 1 else 0)
      | .int bits => (1, u64Signed bits)
      | _         => (2, 0)
    | none => (2, 0)

/-- `h1`'s key ≤ `h2`'s key in canonical order (lexicographic on `scalarOrdKey`). -/
def keyLe (s : HeapState) (h1 h2 : UInt32) : Bool :=
  let (r1, p1) := s.scalarOrdKey h1
  let (r2, p2) := s.scalarOrdKey h2
  r1 < r2 || (r1 == r2 && p1 ≤ p2)

/-- Stable insertion of `x` into a key-sorted list (by `keyLe` on the extracted key handle). -/
def sortInsBy {α : Type} (s : HeapState) (key : α → UInt32) (x : α) : List α → List α
  | []      => [x]
  | y :: ys => if s.keyLe (key x) (key y) then x :: y :: ys else y :: s.sortInsBy key x ys

/-- Sort by canonical key order (stable insertion sort; the keys are unique so stability is moot). -/
def sortBy {α : Type} (s : HeapState) (key : α → UInt32) (l : List α) : List α :=
  l.foldr (fun x acc => s.sortInsBy key x acc) []

/-- `map-to-list(m, desc) → List (Tuple k v)` [BORROWS m + desc]: the entries as fresh 2-element `[k, v]`
arrays in canonical KEY order, wrapped as a list. Each `k`/`v` is dup'd (the tuple co-owns alongside the
still-live map). Empty map → empty list. -/
def mapToList : HeapState → List Value → HeapResult
  | s, [.i32 m, .i32 _desc] =>
    match s.getObj? m with
    | none   => .trap s!"map-to-list: unknown handle {m}"
    | some o =>
      if !o.live then .trap s!"map-to-list: use-after-free (handle {m} freed)"
      else match o.value with
        | .map entries =>
          let pairs := (List.range (entries.size / 2)).map
            (fun i => (entries[2 * i]!, entries[2 * i + 1]!))
          let sorted := s.sortBy Prod.fst pairs
          let (tuples, s') := sorted.foldl
            (fun (acc : Array UInt32 × HeapState) (kv : UInt32 × UInt32) =>
              let (arr, st) := acc
              let st1 := (st.dupH kv.1).dupH kv.2
              let (tup, st2) := st1.alloc (.array #[kv.1, kv.2])
              (arr.push tup, st2))
            (#[], s)
          let (listH, s'') := s'.alloc (.vec tuples)
          .ret [.i32 listH] s''
        | _ => .trap s!"map-to-list: handle {m} is not a map"
  | s, _ => .trap "map-to-list: expected (i32, i32)"

/-- `set-to-list(s, desc) → List a` [BORROWS s + desc]: the elements in canonical VALUE order, each dup'd into
a fresh list (co-owned alongside the still-live set). Empty set → empty list. -/
def setToList : HeapState → List Value → HeapResult
  | s, [.i32 st0, .i32 _desc] =>
    match s.getObj? st0 with
    | none   => .trap s!"set-to-list: unknown handle {st0}"
    | some o =>
      if !o.live then .trap s!"set-to-list: use-after-free (handle {st0} freed)"
      else match o.value with
        | .set elems =>
          let sorted := s.sortBy id elems.toList
          let (arr, s') := sorted.foldl
            (fun (acc : Array UInt32 × HeapState) (e : UInt32) =>
              let (a, stt) := acc
              (a.push e, stt.dupH e))
            (#[], s)
          let (listH, s'') := s'.alloc (.vec arr)
          .ret [.i32 listH] s''
        | _ => .trap s!"set-to-list: handle {st0} is not a set"
  | s, _ => .trap "set-to-list: expected (i32, i32)"

/-! ### Bytes + strings (W5.3a) — a packed immutable byte buffer (runtime.wit 13–16 + `bytes-scalar-at` +
`str-from-bytes` 85). A `bytes` value is a flat `Array UInt8` with NO child handles (drop frees just the
buffer). String and Bytes SHARE this ONE heap representation (per v-runtime: the Str-vs-Bytes distinction is a
value-encode DESCRIPTOR, never a heap-node variant, and no raw op distinguishes them — `str-get` reads the same
buffer as `bytes-get`). `str-from-bytes` UTF-8-validates + re-tags (same handle) or yields NULL; `bytes-scalar-
at` UTF-8-walks to a scalar. The rope ops (`bytes-concat`/`-slice`/`-compact`, with slice-pins-parent sharing)
are W5.3b. `str-new`/`str-get` are NOT emitted (lowerable:false → String is built from bytes);
`str-nfc-normalize` stays unmodeled (a module using it skips, sound). -/

/-- `bytes-alloc(len) → buf`: a fresh byte buffer of `len` zero bytes (rc 1; heap even when empty). -/
def bytesAlloc : HeapState → List Value → HeapResult
  | s, [.i32 len] => s.box (.bytes ((List.replicate len.toNat (0 : UInt8)).toArray))
  | s, _          => .trap "bytes-alloc: expected (i32)"

/-- `bytes-set(buf, i, val) → buf`: store byte `val` (low 8 bits; the compiler guarantees 0–255) at index `i`,
threading the buffer handle back. OOB traps. -/
def bytesSet : HeapState → List Value → HeapResult
  | s, [.i32 buf, .i32 i, .i32 val] =>
    match s.getObj? buf with
    | none   => .trap s!"bytes-set: unknown handle {buf}"
    | some o =>
      if !o.live then .trap s!"bytes-set: use-after-free (handle {buf} freed)"
      else match o.value with
        | .bytes bs =>
          if i.toNat < bs.size then
            .ret [.i32 buf] (s.setObj buf { o with value := .bytes (bs.set! i.toNat val.toUInt8) })
          else .trap s!"bytes-set: index {i} out of bounds (len {bs.size})"
        | _ => .trap s!"bytes-set: handle {buf} is not a byte buffer"
  | s, _ => .trap "bytes-set: expected (i32, i32, i32)"

/-- `bytes-get(buf, i) → val`: the byte at index `i` as a u32 (BORROWS). OOB traps. -/
def bytesGet : HeapState → List Value → HeapResult
  | s, [.i32 buf, .i32 i] =>
    match s.getObj? buf with
    | none   => .trap s!"bytes-get: unknown handle {buf}"
    | some o =>
      if !o.live then .trap s!"bytes-get: use-after-free (handle {buf} freed)"
      else match o.value with
        | .bytes bs =>
          match bs[i.toNat]? with
          | some b => .ret [.i32 b.toUInt32] s
          | none   => .trap s!"bytes-get: index {i} out of bounds (len {bs.size})"
        | _ => .trap s!"bytes-get: handle {buf} is not a byte buffer"
  | s, _ => .trap "bytes-get: expected (i32, i32)"

/-- `bytes-len(buf) → len`: the byte count. -/
def bytesLen : HeapState → List Value → HeapResult
  | s, [.i32 buf] =>
    match s.getObj? buf with
    | none   => .trap s!"bytes-len: unknown handle {buf}"
    | some o =>
      if !o.live then .trap s!"bytes-len: use-after-free (handle {buf} freed)"
      else match o.value with
        | .bytes bs => .ret [.i32 bs.size.toUInt32] s
        | _         => .trap s!"bytes-len: handle {buf} is not a byte buffer"
  | s, _ => .trap "bytes-len: expected (i32)"

/-- The UTF-8 scalars (codepoints) of a byte buffer, or `none` if ill-formed — via the stdlib validator. -/
def utf8Scalars? (bs : Array UInt8) : Option (List UInt32) :=
  (String.fromUTF8? (ByteArray.mk bs)).map (fun str => str.toList.map (fun c => c.val))

/-- `bytes-scalar-at(buf, i) → codepoint`: the `i`-th Unicode SCALAR of the buffer's UTF-8; an out-of-range or
ill-formed index returns `0xFFFFFFFF` (the compiler maps that to `None` in `(Option Char)`). BORROWS. -/
def bytesScalarAt : HeapState → List Value → HeapResult
  | s, [.i32 buf, .i32 i] =>
    if isImmediate buf then .ret [.i32 0xFFFFFFFF] s
    else match s.getObj? buf with
    | none   => .trap s!"bytes-scalar-at: unknown handle {buf}"
    | some o =>
      if !o.live then .trap s!"bytes-scalar-at: use-after-free (handle {buf} freed)"
      else match o.value with
        | .bytes bs =>
          match utf8Scalars? bs with
          | some cps => .ret [.i32 ((cps[i.toNat]?).getD 0xFFFFFFFF)] s
          | none     => .ret [.i32 0xFFFFFFFF] s
        | _ => .trap s!"bytes-scalar-at: handle {buf} is not a byte buffer"
  | s, _ => .trap "bytes-scalar-at: expected (i32, i32)"

/-- `str-from-bytes(buf) → str | NULL` [CONSUMES buf]: STRICT UTF-8 validate. VALID → the SAME handle re-tagged
String (String and Bytes are one heap rep, so this is identity; ownership moves buf→result, rc unchanged).
INVALID → drop buf, return NULL (`0`); the compiler wraps the result into `Option` (NULL→None). An immediate
buf is a fine empty string → returned as-is. -/
def strFromBytes : HeapState → List Value → HeapResult
  | s, [.i32 buf] =>
    if isImmediate buf then .ret [.i32 buf] s
    else match s.getObj? buf with
    | none   => .trap s!"str-from-bytes: unknown handle {buf}"
    | some o =>
      if !o.live then .trap s!"str-from-bytes: use-after-free (handle {buf} freed)"
      else match o.value with
        | .bytes bs =>
          match String.fromUTF8? (ByteArray.mk bs) with
          | some _ => .ret [.i32 buf] s            -- valid: same handle becomes the String (consumed→returned)
          | none   => .ret [.i32 0] (s.dropH buf)  -- invalid UTF-8: drop the buffer, return NULL
        | _ => .trap s!"str-from-bytes: handle {buf} is not a byte buffer"
  | s, _ => .trap "str-from-bytes: expected (i32)"

/-! ### Bytes rope (W5.3b) — `bytes-concat`/`-slice`/`-compact`. The runtime uses a persistent rope (O(1)
concat/slice that SHARE leaves — a slice pins its parent alive), but the value form is INDISTINGUISHABLE from a
flat buffer by `bytes-len`/`-get`/equality (runtime.wit), so a FLAT model (materialize the bytes) is
value-faithful. It is ALSO leak-VERDICT-faithful: the leak oracle asserts `liveCount == 0` (leak vs no-leak),
NOT an exact count, and a flat model frees operands eagerly such that its census is 0 EXACTLY when the rope's
is — a concat/slice result is dropped ⇔ the rope's pinned operand(s) are freed, and the flat buffer is freed on
that same drop. So the pinned-parent sharing changes only the leak COUNT on an already-leaking run, never the
0-vs-nonzero VERDICT. `concat`/`slice` CONSUME their operand(s); `compact` is identity here (the flat buffer is
already storage-independent). -/

/-- `bytes-concat(a, b) → buf` [CONSUMES a, b]: a fresh buffer = a's bytes then b's (empty is the identity). -/
def bytesConcat : HeapState → List Value → HeapResult
  | s, [.i32 a, .i32 b] =>
    match s.getObj? a, s.getObj? b with
    | none, _ => .trap s!"bytes-concat: unknown handle {a}"
    | _, none => .trap s!"bytes-concat: unknown handle {b}"
    | some oa, some ob =>
      if !oa.live then .trap s!"bytes-concat: use-after-free (handle {a} freed)"
      else if !ob.live then .trap s!"bytes-concat: use-after-free (handle {b} freed)"
      else match oa.value, ob.value with
        | .bytes ba, .bytes bb =>
          let (r, s1) := s.alloc (.bytes (ba ++ bb))
          .ret [.i32 r] ((s1.dropH a).dropH b)
        | _, _ => .trap s!"bytes-concat: handle {a} or {b} is not a byte buffer"
  | s, _ => .trap "bytes-concat: expected (i32, i32)"

/-- `bytes-slice(buf, start, len) → buf'` [CONSUMES buf]: `len` bytes from `start`; total-or-trap
(`start + len > bytes-len` traps; `len == 0` is the empty buffer). Flat model — see the section note on why
this is leak-verdict-equivalent to the runtime's parent-pinning slice. -/
def bytesSlice : HeapState → List Value → HeapResult
  | s, [.i32 buf, .i32 start, .i32 len] =>
    match s.getObj? buf with
    | none => .trap s!"bytes-slice: unknown handle {buf}"
    | some o =>
      if !o.live then .trap s!"bytes-slice: use-after-free (handle {buf} freed)"
      else match o.value with
        | .bytes bs =>
          if start.toNat + len.toNat ≤ bs.size then
            let sub := ((bs.toList.drop start.toNat).take len.toNat).toArray
            let (r, s1) := s.alloc (.bytes sub)
            .ret [.i32 r] (s1.dropH buf)
          else .trap s!"bytes-slice: [{start}, {start}+{len}) out of bounds (len {bs.size})"
        | _ => .trap s!"bytes-slice: handle {buf} is not a byte buffer"
  | s, _ => .trap "bytes-slice: expected (i32, i32, i32)"

/-- `bytes-compact(buf) → buf` [CONSUMES buf]: a content-equal, storage-independent buffer. In the flat model
the buffer is ALREADY an independent leaf, so this is identity — the same handle (ownership moves in→out). -/
def bytesCompact : HeapState → List Value → HeapResult
  | s, [.i32 buf] =>
    match s.getObj? buf with
    | none => .trap s!"bytes-compact: unknown handle {buf}"
    | some o =>
      if !o.live then .trap s!"bytes-compact: use-after-free (handle {buf} freed)"
      else match o.value with
        | .bytes _ => .ret [.i32 buf] s
        | _        => .trap s!"bytes-compact: handle {buf} is not a byte buffer"
  | s, _ => .trap "bytes-compact: expected (i32)"

/-! ### Sums (tagged variants, indices 10–12) — a discriminant (u32; the compiler's per-sum-type variant index
0,1,2,… over the variants in declaration order, NOT a universal tag) + a payload handle. A NULLARY variant
carries the unit value (`arr-alloc(0)` → the unit immediate) as its payload. Constructors consume, accessors
borrow (value-heap-runtime.md), mirroring arrays: `sum-new` MOVES the payload into the node (the sum owns it;
drop cascades into it — arity 1); `sum-disc`/`sum-payload` BORROW. -/

/-- `sum-new(disc, payload) → handle` [CONSUMES payload]: a fresh sum node tagged `disc`, owning `payload`. -/
def sumNew : HeapState → List Value → HeapResult
  | s, [.i32 disc, .i32 payload] => s.box (.sum disc payload)
  | s, _ => .trap "sum-new: expected (i32, i32)"

/-- `sum-disc(h) → disc`: the variant discriminant (BORROWS). -/
def sumDisc : HeapState → List Value → HeapResult
  | s, [.i32 h] =>
    match s.getObj? h with
    | none   => .trap s!"sum-disc: unknown handle {h}"
    | some o =>
      if !o.live then .trap s!"sum-disc: use-after-free (handle {h} freed)"
      else match o.value with
        | .sum d _ => .ret [.i32 d] s
        | _        => .trap s!"sum-disc: handle {h} is not a sum"
  | s, _ => .trap "sum-disc: expected (i32)"

/-- `sum-payload(h) → payload`: the payload handle, BORROWED (rc unchanged; the sum keeps ownership — a caller
that keeps it gets a compiler-emitted `dup`, like `arr-get`). -/
def sumPayload : HeapState → List Value → HeapResult
  | s, [.i32 h] =>
    match s.getObj? h with
    | none   => .trap s!"sum-payload: unknown handle {h}"
    | some o =>
      if !o.live then .trap s!"sum-payload: use-after-free (handle {h} freed)"
      else match o.value with
        | .sum _ p => .ret [.i32 p] s
        | _        => .trap s!"sum-payload: handle {h} is not a sum"
  | s, _ => .trap "sum-payload: expected (i32)"

/-! ### Arbitrary-precision integer (BigInt, indices 65–73) — a sign-magnitude leaf (zero children), modeled
as a Lean `Int`. Per v-runtime (scalars.rs): a BigInt is ALWAYS a heap leaf (never a fixnum immediate; and
zero is NOT canonicalized to null — construction always allocs a FRESH heap zero-leaf, census-counted, the
compiler emits its drop). A null/missing handle READS as canonical zero (a defensive scalar-read tolerance,
never a trap), but construction never produces null; two zeros are DISTINCT leaves, equal by structural
byte-eq. Ownership is BORROW-heavy (the OPPOSITE of the CHAMP collection ops): every arith/cmp/convert BORROWS
its operand(s) and boxes a FRESH owned result — the caller drops the operands. `bigint-div`/`-rem` truncate
toward zero and TRAP on a zero divisor. `bigint-of-bytes` (constant materialization) + Rational are a follow-up. -/

/-- Allocate a heap BigInt leaf — ALWAYS heap, even for zero (no null canonicalization). -/
def mkBigInt (s : HeapState) (v : Int) : UInt32 × HeapState := s.alloc (.bigint v)

/-- Read a BigInt operand: a null/missing handle is canonical zero (read tolerance); a live `.bigint` leaf is
its value; anything else → `none` (the caller traps). -/
def bigintVal? (s : HeapState) (h : UInt32) : Option Int :=
  if h == 0 then some 0
  else match s.getObj? h with
    | some o => if o.live then (match o.value with | .bigint v => some v | _ => none) else none
    | none   => none

/-- `bigint-of-i64(v) → handle`: widen a signed i64 into a fresh BigInt leaf. -/
def bigintOfI64 : HeapState → List Value → HeapResult
  | s, [.i64 n] => let (r, s') := s.mkBigInt (u64Signed n); .ret [.i32 r] s'
  | s, _        => .trap "bigint-of-i64: expected (i64)"

/-- `bigint-to-i64-checked(h) → i64`: narrow back (BORROWS); TRAPS if out of signed-i64 range. -/
def bigintToI64Checked : HeapState → List Value → HeapResult
  | s, [.i32 h] =>
    match s.bigintVal? h with
    | none   => .trap s!"bigint-to-i64-checked: handle {h} is not a bigint"
    | some v =>
      if (-9223372036854775808 : Int) ≤ v && v ≤ (9223372036854775807 : Int) then .ret [.i64 (intToU64Bits v)] s
      else .trap "bigint-to-i64-checked: value out of i64 range"
  | s, _ => .trap "bigint-to-i64-checked: expected (i32)"

/-- The shared shape of a binary BigInt arith op: BORROW a, b; box a FRESH result via `op v_a v_b`. -/
def bigintBin (s : HeapState) (op : Int → Int → Int) (a b : UInt32) : HeapResult :=
  match s.bigintVal? a, s.bigintVal? b with
  | some x, some y => let (r, s') := s.mkBigInt (op x y); .ret [.i32 r] s'
  | _, _           => .trap "bigint arith: an operand is not a bigint"

/-- `bigint-add(a,b)` — BORROWS both, fresh result. -/
def bigintAdd : HeapState → List Value → HeapResult
  | s, [.i32 a, .i32 b] => s.bigintBin (· + ·) a b
  | s, _ => .trap "bigint-add: expected (i32, i32)"
/-- `bigint-sub(a,b)`. -/
def bigintSub : HeapState → List Value → HeapResult
  | s, [.i32 a, .i32 b] => s.bigintBin (· - ·) a b
  | s, _ => .trap "bigint-sub: expected (i32, i32)"
/-- `bigint-mul(a,b)`. -/
def bigintMul : HeapState → List Value → HeapResult
  | s, [.i32 a, .i32 b] => s.bigintBin (· * ·) a b
  | s, _ => .trap "bigint-mul: expected (i32, i32)"

/-- `bigint-div(a,b)`: truncate toward zero; TRAP on a zero divisor. -/
def bigintDiv : HeapState → List Value → HeapResult
  | s, [.i32 a, .i32 b] =>
    match s.bigintVal? a, s.bigintVal? b with
    | some x, some y =>
      if y == 0 then .trap "bigint-div: division by zero"
      else let (r, s') := s.mkBigInt (Int.tdiv x y); .ret [.i32 r] s'
    | _, _ => .trap "bigint-div: an operand is not a bigint"
  | s, _ => .trap "bigint-div: expected (i32, i32)"

/-- `bigint-rem(a,b)` = a % b (remainder of truncating division, DIVIDEND's sign); TRAP on a zero divisor. -/
def bigintRem : HeapState → List Value → HeapResult
  | s, [.i32 a, .i32 b] =>
    match s.bigintVal? a, s.bigintVal? b with
    | some x, some y =>
      if y == 0 then .trap "bigint-rem: division by zero"
      else let (r, s') := s.mkBigInt (Int.tmod x y); .ret [.i32 r] s'
    | _, _ => .trap "bigint-rem: an operand is not a bigint"
  | s, _ => .trap "bigint-rem: expected (i32, i32)"

/-- `bigint-cmp(a,b) → -1|0|1`: three-way compare (BORROWS). -/
def bigintCmp : HeapState → List Value → HeapResult
  | s, [.i32 a, .i32 b] =>
    match s.bigintVal? a, s.bigintVal? b with
    | some x, some y => .ret [.i64 (intToU64Bits (if x < y then -1 else if x == y then 0 else 1))] s
    | _, _ => .trap "bigint-cmp: an operand is not a bigint"
  | s, _ => .trap "bigint-cmp: expected (i32, i32)"

/-! ### Exact rational (indices 74–81) — a NORMALIZED 2-handle node `[num, den]`, each child a BigInt leaf
(lowest terms, sign on the numerator, den > 0). Per v-runtime (scalars.rs): `rational-of` CONSUMES its two
BigInt operands and normalizes (gcd-reduce, sign→num, den>0; TRAP on den=0), so 2/4 and 1/2 are the SAME node;
`rational-num`/`-den` BORROW the rational and return an OWNED (dup'd) handle to the normalized child; the arith
+ `rational-cmp` BORROW both operands and box a FRESH result (same borrow-heavy discipline as BigInt). A
rational is an ordinary node — dup/drop recurse into BOTH children. -/

/-- Normalize `(x, y)` (y ≠ 0) to lowest terms with den > 0 (sign on the numerator). -/
def normRat (x y : Int) : Int × Int :=
  let (x, y) := if y < 0 then (-x, -y) else (x, y)
  let g : Int := (Int.gcd x y : Nat)   -- gcd of |x|,|y|; ≥ 1 since y ≠ 0 (gcd 0 y = |y|)
  (x / g, y / g)

/-- Allocate a normalized rational node `[num, den]` (two fresh BigInt leaves + the rational). Caller ensures
`y ≠ 0`. -/
def mkRational (s : HeapState) (x y : Int) : UInt32 × HeapState :=
  let (n, d) := normRat x y
  let (nh, s1) := s.mkBigInt n
  let (dh, s2) := s1.mkBigInt d
  s2.alloc (.rational nh dh)

/-- The (numerator, denominator) VALUES of a live rational node (reading its BigInt children). -/
def ratComponents? (s : HeapState) (r : UInt32) : Option (Int × Int) :=
  match s.getObj? r with
  | some o =>
    if o.live then
      match o.value with
      | .rational nh dh =>
        match s.bigintVal? nh, s.bigintVal? dh with
        | some x, some y => some (x, y)
        | _, _           => none
      | _ => none
    else none
  | none => none

/-- `rational-of(num, den) → r` [CONSUMES num, den]: normalize `(num, den)` to lowest terms; TRAP on den = 0. -/
def rationalOf : HeapState → List Value → HeapResult
  | s, [.i32 num, .i32 den] =>
    match s.bigintVal? num, s.bigintVal? den with
    | some x, some y =>
      if y == 0 then .trap "rational-of: zero denominator"
      else
        let (r, s1) := s.mkRational x y
        .ret [.i32 r] ((s1.dropH num).dropH den)
    | _, _ => .trap "rational-of: an operand is not a bigint"
  | s, _ => .trap "rational-of: expected (i32, i32)"

/-- `rational-num(r) → num`: the numerator (a fresh OWNED handle, dup'd from the normalized child; BORROWS r). -/
def rationalNum : HeapState → List Value → HeapResult
  | s, [.i32 r] =>
    match s.getObj? r with
    | none   => .trap s!"rational-num: unknown handle {r}"
    | some o =>
      if !o.live then .trap s!"rational-num: use-after-free (handle {r} freed)"
      else match o.value with
        | .rational nh _ => .ret [.i32 nh] (s.dupH nh)
        | _              => .trap s!"rational-num: handle {r} is not a rational"
  | s, _ => .trap "rational-num: expected (i32)"

/-- `rational-den(r) → den`: the denominator (a fresh OWNED handle; BORROWS r). -/
def rationalDen : HeapState → List Value → HeapResult
  | s, [.i32 r] =>
    match s.getObj? r with
    | none   => .trap s!"rational-den: unknown handle {r}"
    | some o =>
      if !o.live then .trap s!"rational-den: use-after-free (handle {r} freed)"
      else match o.value with
        | .rational _ dh => .ret [.i32 dh] (s.dupH dh)
        | _              => .trap s!"rational-den: handle {r} is not a rational"
  | s, _ => .trap "rational-den: expected (i32)"

/-- The shared shape of a binary rational arith op: BORROW a, b; box a FRESH normalized result from the
component values via `op`. `op (xa,ya) (xb,yb)` returns the un-normalized `(num, den)`. -/
def rationalBin (s : HeapState) (op : Int × Int → Int × Int → Int × Int) (a b : UInt32) : HeapResult :=
  match s.ratComponents? a, s.ratComponents? b with
  | some pa, some pb =>
    let (n, d) := op pa pb
    if d == 0 then .trap "rational arith: zero denominator"
    else let (r, s') := s.mkRational n d; .ret [.i32 r] s'
  | _, _ => .trap "rational arith: an operand is not a rational"

/-- `rational-add(a,b)` = (xa·yb + xb·ya)/(ya·yb) — BORROWS both, fresh normalized result. -/
def rationalAdd : HeapState → List Value → HeapResult
  | s, [.i32 a, .i32 b] => s.rationalBin (fun (xa, ya) (xb, yb) => (xa * yb + xb * ya, ya * yb)) a b
  | s, _ => .trap "rational-add: expected (i32, i32)"
/-- `rational-sub(a,b)` = (xa·yb − xb·ya)/(ya·yb). -/
def rationalSub : HeapState → List Value → HeapResult
  | s, [.i32 a, .i32 b] => s.rationalBin (fun (xa, ya) (xb, yb) => (xa * yb - xb * ya, ya * yb)) a b
  | s, _ => .trap "rational-sub: expected (i32, i32)"
/-- `rational-mul(a,b)` = (xa·xb)/(ya·yb). -/
def rationalMul : HeapState → List Value → HeapResult
  | s, [.i32 a, .i32 b] => s.rationalBin (fun (xa, ya) (xb, yb) => (xa * xb, ya * yb)) a b
  | s, _ => .trap "rational-mul: expected (i32, i32)"
/-- `rational-div(a,b)` = (xa·yb)/(ya·xb) — TRAPS on a zero divisor (xb = 0). -/
def rationalDiv : HeapState → List Value → HeapResult
  | s, [.i32 a, .i32 b] => s.rationalBin (fun (xa, ya) (xb, yb) => (xa * yb, ya * xb)) a b
  | s, _ => .trap "rational-div: expected (i32, i32)"

/-- `rational-cmp(a,b) → -1|0|1`: compare xa·yb vs xb·ya (both dens > 0); BORROWS. -/
def rationalCmp : HeapState → List Value → HeapResult
  | s, [.i32 a, .i32 b] =>
    match s.ratComponents? a, s.ratComponents? b with
    | some (xa, ya), some (xb, yb) =>
      let l := xa * yb; let r := xb * ya
      .ret [.i64 (intToU64Bits (if l < r then -1 else if l == r then 0 else 1))] s
    | _, _ => .trap "rational-cmp: an operand is not a rational"
  | s, _ => .trap "rational-cmp: expected (i32, i32)"

end HeapState

/-! ### HostFn wrappers + the name-keyed table W5.1c turns into a `HostRegistry`. -/

/-- Lift a `HeapState` op into a talos `HostFn` over `Store HeapState`: run it on the store's host slot,
then reflect the result / trap back into the store. `params`/`results` are the declared core signature
(the interpreter trusts them; they let W5.1c's registry match the emitted `"heap"` import decls). -/
def toHostFn (params results : List ValueType)
    (run : HeapState → List Value → HeapResult) : HostFn HeapState where
  params := params
  results := results
  invoke := fun st args =>
    match run st.host args with
    | .ret vals h' => .Return vals { st with host := h' }
    | .trap m      => .Trap st m

/-- The modeled `"heap"` ops, keyed by their exact emitted kebab-case name + core signature (from
`rcdzc/src/backend/wasm/runtime_abi.rs`, core-valtype-projected: U32/Bool→i32, S64→i64, F64→f64, F32→f32).
W5.1c builds a `HostRegistry` from this by pairing each with an `ImportDecl { «module» := "heap", name,
params, results }` (talos resolves the emitted `(type N)` import sig, so the params/results match); later
increments extend the table. -/
def heapHostOps : List (String × HostFn HeapState) :=
  [ -- refcount / liveness core
    ("dup",                toHostFn [.i32] []      HeapState.dup)
  , ("drop",               toHostFn [.i32] []      HeapState.drop)
  , ("live-objects",       toHostFn []     [.i32]  HeapState.liveObjects)
    -- boxing
  , ("box-int",            toHostFn [.i64] [.i32]  HeapState.boxInt)
  , ("box-float",          toHostFn [.f64] [.i32]  HeapState.boxFloat)
  , ("box-float32",        toHostFn [.f32] [.i32]  HeapState.boxFloat32)
  , ("box-bool",           toHostFn [.i32] [.i32]  HeapState.boxBool)
  , ("get-int",            toHostFn [.i32] [.i64]  HeapState.getInt)
  , ("get-float",          toHostFn [.i32] [.f64]  HeapState.getFloat)
  , ("get-float32",        toHostFn [.i32] [.f32]  HeapState.getFloat32)
  , ("get-bool",           toHostFn [.i32] [.i32]  HeapState.getBool)
    -- arrays
  , ("arr-alloc",          toHostFn [.i32]             [.i32]  HeapState.arrAlloc)
  , ("arr-set",            toHostFn [.i32, .i32, .i32] [.i32]  HeapState.arrSet)
  , ("arr-get",            toHostFn [.i32, .i32]       [.i32]  HeapState.arrGet)
  , ("arr-len",            toHostFn [.i32]             [.i32]  HeapState.arrLen)
    -- maps (positional literal form; functional HAMT insert/lookup/… is a later slice)
  , ("map-alloc",          toHostFn [.i32]                   [.i32]  HeapState.mapAlloc)
  , ("map-set",            toHostFn [.i32, .i32, .i32, .i32] [.i32]  HeapState.mapSet)
  , ("map-key",            toHostFn [.i32, .i32]             [.i32]  HeapState.mapKey)
  , ("map-val",            toHostFn [.i32, .i32]             [.i32]  HeapState.mapVal)
  , ("map-len",            toHostFn [.i32]                   [.i32]  HeapState.mapLen)
    -- functional map, read-only (consuming insert/remove/merge = W5.2b-2; iter/to-list = W5.2c)
  , ("map-empty",          toHostFn []                       [.i32]  HeapState.mapEmpty)
  , ("map-lookup",         toHostFn [.i32, .i32]             [.i32]  HeapState.mapLookup)
  , ("map-size",           toHostFn [.i32]                   [.i32]  HeapState.mapSize)
  , ("map-insert",         toHostFn [.i32, .i32, .i32]       [.i32]  HeapState.mapInsert)
  , ("map-remove",         toHostFn [.i32, .i32]             [.i32]  HeapState.mapRemove)
  , ("map-merge",          toHostFn [.i32, .i32]             [.i32]  HeapState.mapMerge)
    -- sets, core (union/intersection/difference = W5.2d-2; iter/to-list = W5.2c)
  , ("set-empty",          toHostFn []                       [.i32]  HeapState.setEmpty)
  , ("set-insert",         toHostFn [.i32, .i32]             [.i32]  HeapState.setInsert)
  , ("set-contains",       toHostFn [.i32, .i32]             [.i32]  HeapState.setContains)
  , ("set-remove",         toHostFn [.i32, .i32]             [.i32]  HeapState.setRemove)
  , ("set-size",           toHostFn [.i32]                   [.i32]  HeapState.setSize)
  , ("set-union",          toHostFn [.i32, .i32]             [.i32]  HeapState.setUnion)
  , ("set-intersection",   toHostFn [.i32, .i32]             [.i32]  HeapState.setIntersection)
  , ("set-difference",     toHostFn [.i32, .i32]             [.i32]  HeapState.setDifference)
    -- map/set enumeration (W5.2c): the program-observable, canonical value-sorted `to-list`
  , ("map-to-list",        toHostFn [.i32, .i32]             [.i32]  HeapState.mapToList)
  , ("set-to-list",        toHostFn [.i32, .i32]             [.i32]  HeapState.setToList)
    -- bytes + strings (W5.3a): packed byte buffer + UTF-8; concat/slice/compact rope = W5.3b
  , ("bytes-alloc",        toHostFn [.i32]                   [.i32]  HeapState.bytesAlloc)
  , ("bytes-set",          toHostFn [.i32, .i32, .i32]       [.i32]  HeapState.bytesSet)
  , ("bytes-get",          toHostFn [.i32, .i32]             [.i32]  HeapState.bytesGet)
  , ("bytes-len",          toHostFn [.i32]                   [.i32]  HeapState.bytesLen)
  , ("bytes-scalar-at",    toHostFn [.i32, .i32]             [.i32]  HeapState.bytesScalarAt)
  , ("str-from-bytes",     toHostFn [.i32]                   [.i32]  HeapState.strFromBytes)
    -- bytes rope (W5.3b): concat/slice/compact, flat model
  , ("bytes-concat",       toHostFn [.i32, .i32]             [.i32]  HeapState.bytesConcat)
  , ("bytes-slice",        toHostFn [.i32, .i32, .i32]       [.i32]  HeapState.bytesSlice)
  , ("bytes-compact",      toHostFn [.i32]                   [.i32]  HeapState.bytesCompact)
    -- sums (tagged variants): construct (consume payload) + disc/payload accessors (borrow)
  , ("sum-new",            toHostFn [.i32, .i32]             [.i32]  HeapState.sumNew)
  , ("sum-disc",           toHostFn [.i32]                   [.i32]  HeapState.sumDisc)
  , ("sum-payload",        toHostFn [.i32]                   [.i32]  HeapState.sumPayload)
    -- arbitrary-precision integer (BigInt): borrow-heavy — arith/cmp/convert BORROW + fresh owned result
  , ("bigint-of-i64",         toHostFn [.i64]                [.i32]  HeapState.bigintOfI64)
  , ("bigint-to-i64-checked", toHostFn [.i32]                [.i64]  HeapState.bigintToI64Checked)
  , ("bigint-add",            toHostFn [.i32, .i32]          [.i32]  HeapState.bigintAdd)
  , ("bigint-sub",            toHostFn [.i32, .i32]          [.i32]  HeapState.bigintSub)
  , ("bigint-mul",            toHostFn [.i32, .i32]          [.i32]  HeapState.bigintMul)
  , ("bigint-div",            toHostFn [.i32, .i32]          [.i32]  HeapState.bigintDiv)
  , ("bigint-rem",            toHostFn [.i32, .i32]          [.i32]  HeapState.bigintRem)
  , ("bigint-cmp",            toHostFn [.i32, .i32]          [.i64]  HeapState.bigintCmp)
    -- exact rational: rational-of consumes+normalizes; num/den + arith/cmp borrow (fresh owned result)
  , ("rational-of",           toHostFn [.i32, .i32]          [.i32]  HeapState.rationalOf)
  , ("rational-num",          toHostFn [.i32]                [.i32]  HeapState.rationalNum)
  , ("rational-den",          toHostFn [.i32]                [.i32]  HeapState.rationalDen)
  , ("rational-add",          toHostFn [.i32, .i32]          [.i32]  HeapState.rationalAdd)
  , ("rational-sub",          toHostFn [.i32, .i32]          [.i32]  HeapState.rationalSub)
  , ("rational-mul",          toHostFn [.i32, .i32]          [.i32]  HeapState.rationalMul)
  , ("rational-div",          toHostFn [.i32, .i32]          [.i32]  HeapState.rationalDiv)
  , ("rational-cmp",          toHostFn [.i32, .i32]          [.i64]  HeapState.rationalCmp)
    -- lists (vec-*, growable sequence) — core + the extra constructors (concat/prepend/of-arr/drop)
  , ("vec-empty",          toHostFn []                       [.i32]  HeapState.vecEmpty)
  , ("vec-len",            toHostFn [.i32]                   [.i32]  HeapState.vecLen)
  , ("vec-get",            toHostFn [.i32, .i32]             [.i32]  HeapState.vecGet)
  , ("vec-push",           toHostFn [.i32, .i32]             [.i32]  HeapState.vecPush)
  , ("vec-update",         toHostFn [.i32, .i32, .i32]       [.i32]  HeapState.vecUpdate)
  , ("vec-concat",         toHostFn [.i32, .i32]             [.i32]  HeapState.vecConcat)
  , ("vec-prepend",        toHostFn [.i32, .i32]             [.i32]  HeapState.vecPrepend)
  , ("vec-of-arr",         toHostFn [.i32]                   [.i32]  HeapState.vecOfArr)
  , ("vec-drop",           toHostFn [.i32, .i32]             [.i32]  HeapState.vecDrop)
    -- immortality
  , ("mark-immortal",      toHostFn [.i32] [.i32]  HeapState.markImmortal)
  , ("mark-immortal-deep", toHostFn [.i32] [.i32]  HeapState.markImmortalDeep) ]

/-! ### Witnesses — compiled every build (a regression fails the oracle-lean build). Immediate-aware after the
IMMEDIATES rework: box-int(fixnum)/box-bool produce inline immediates (no heap, census-excluded, dup/drop
no-op); a value that must be a HEAP object for a refcount/cascade test uses box-float (floats never inline).
This focused set proves the immediates model; the collection consume-op witnesses are the
`probeMapMerge`/`probeSet{Union,Intersection,Difference}` witnesses at the bottom of this section. -/

open HeapState

/-- box-int of a fixnum inlines as an IMMEDIATE: no heap object (liveCount 0), and get-int round-trips. -/
private def probeImmInt : Bool :=
  match boxInt ({} : HeapState) [.i64 5] with
  | .ret [.i32 h] s =>
    isImmediate h && (s.liveCount == 0) &&
    (match getInt s [.i32 h] with | .ret [.i64 5] _ => true | _ => false)
  | _ => false
example : probeImmInt = true := by native_decide

/-- A NEGATIVE fixnum immediate round-trips (exercises the signed decode). -/
private def probeImmIntNeg : Bool :=
  match boxInt ({} : HeapState) [.i64 0xFFFFFFFFFFFFFFFD] with
  | .ret [.i32 h] s =>
    isImmediate h &&
    (match getInt s [.i32 h] with | .ret [.i64 n] _ => n == 0xFFFFFFFFFFFFFFFD | _ => false)
  | _ => false
example : probeImmIntNeg = true := by native_decide

/-- The LEAK fix: boxing a fixnum (immediate) without dropping leaves NO live heap object → no false leak. -/
private def probeImmNoLeak : Bool :=
  match boxInt ({} : HeapState) [.i64 7] with
  | .ret [.i32 _] s => s.liveCount == 0
  | _ => false
example : probeImmNoLeak = true := by native_decide

/-- dup/drop of an immediate are NO-OPS (census stays 0). -/
private def probeImmDupDrop : Bool :=
  match boxInt ({} : HeapState) [.i64 3] with
  | .ret [.i32 h] s =>
    (match dup s [.i32 h] with
     | .ret [] s1 => (match drop s1 [.i32 h] with | .ret [] s2 => s2.liveCount == 0 | _ => false)
     | _          => false)
  | _ => false
example : probeImmDupDrop = true := by native_decide

/-- box-bool inlines as an immediate; get-bool round-trips (5 → true → 1); no heap. -/
private def probeBool : Bool :=
  match boxBool ({} : HeapState) [.i32 5] with
  | .ret [.i32 h] s =>
    isImmediate h && (s.liveCount == 0) &&
    (match getBool s [.i32 h] with | .ret [.i32 1] _ => true | _ => false)
  | _ => false
example : probeBool = true := by native_decide

/-- box-int of a NON-fixnum heap-allocates (liveCount 1); get-int round-trips; drop frees it. -/
private def probeHeapInt : Bool :=
  match boxInt ({} : HeapState) [.i64 1000000000] with
  | .ret [.i32 h] s =>
    (!isImmediate h) && (s.liveCount == 1) &&
    (match getInt s [.i32 h] with | .ret [.i64 1000000000] _ => true | _ => false) &&
    (match drop s [.i32 h] with | .ret [] s2 => s2.liveCount == 0 | _ => false)
  | _ => false
example : probeHeapInt = true := by native_decide

/-- use-after-free / double-free trap on a HEAP object (box-float, drop, then access / re-drop). -/
private def probeUseAfterFree : Bool :=
  match boxFloat ({} : HeapState) [.f64 1] with
  | .ret [.i32 h] s1 =>
    match drop s1 [.i32 h] with
    | .ret [] s2 =>
      (match getFloat s2 [.i32 h] with | .trap _ => true | _ => false) &&
      (match drop s2 [.i32 h]     with | .trap _ => true | _ => false)
    | _ => false
  | _ => false
example : probeUseAfterFree = true := by native_decide

/-- Cascade-drop with HEAP children: an array owns two boxed floats (liveCount 3); dropping it frees all. -/
private def probeHeapCascade : Bool :=
  match boxFloat ({} : HeapState) [.f64 1] with
  | .ret [.i32 f1] s0 =>
    match boxFloat s0 [.f64 2] with
    | .ret [.i32 f2] s1 =>
      match arrAlloc s1 [.i32 2] with
      | .ret [.i32 a] s2 =>
        match arrSet s2 [.i32 a, .i32 0, .i32 f1] with
        | .ret [.i32 _] s3 =>
          match arrSet s3 [.i32 a, .i32 1, .i32 f2] with
          | .ret [.i32 _] s4 =>
            (s4.liveCount == 3) &&
            (match drop s4 [.i32 a] with | .ret [] s5 => s5.liveCount == 0 | _ => false)
          | _ => false
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeHeapCascade = true := by native_decide

/-- An array of IMMEDIATE int elements: only the array node is heap (liveCount 1); arr-get yields the
immediate, get-int reads it; cascade-drop skips the immediate elements and frees just the node → 0. -/
private def probeArrImmElems : Bool :=
  match arrAlloc ({} : HeapState) [.i32 2] with
  | .ret [.i32 a] s0 =>
    match boxInt s0 [.i64 5] with
    | .ret [.i32 e] s1 =>
      match arrSet s1 [.i32 a, .i32 0, .i32 e] with
      | .ret [.i32 _] s2 =>
        (s2.liveCount == 1) &&
        (match arrGet s2 [.i32 a, .i32 0] with
         | .ret [.i32 g] _ => (match getInt s2 [.i32 g] with | .ret [.i64 5] _ => true | _ => false)
         | _               => false) &&
        (match drop s2 [.i32 a] with | .ret [] s3 => s3.liveCount == 0 | _ => false)
      | _ => false
    | _ => false
  | _ => false
example : probeArrImmElems = true := by native_decide

/-- arr-alloc(0) is the inline UNIT immediate: it IS immUnit, no heap (liveCount 0), arr-len 0, arr-get OOB. -/
private def probeImmUnit : Bool :=
  match arrAlloc ({} : HeapState) [.i32 0] with
  | .ret [.i32 u] s =>
    (u == immUnit) && (s.liveCount == 0) &&
    (match arrLen s [.i32 u]         with | .ret [.i32 0] _ => true | _ => false) &&
    (match arrGet s [.i32 u, .i32 0] with | .trap _ => true | _ => false)
  | _ => false
example : probeImmUnit = true := by native_decide

/-- A map with IMMEDIATE int keys (the realistic corpus case): value-eq matches the deterministic immediate
key, lookup returns the heap value, and dropping the map frees the map node + the value (the immediate key is
census-excluded) → 0. This is the fix for the map diverge/leak from small-int keys. -/
private def probeMapImmKeys : Bool :=
  match mapEmpty ({} : HeapState) [] with
  | .ret [.i32 e] s0 =>
    match boxInt s0 [.i64 5] with
    | .ret [.i32 k] s1 =>
      match boxFloat s1 [.f64 9] with
      | .ret [.i32 v] s2 =>
        match mapInsert s2 [.i32 e, .i32 k, .i32 v] with
        | .ret [.i32 m] s3 =>
          match boxInt s3 [.i64 5] with
          | .ret [.i32 k2] s4 =>
            (match mapLookup s4 [.i32 m, .i32 k2] with | .ret [.i32 g] _ => g == v | _ => false) &&
            (match mapSize s4 [.i32 m]             with | .ret [.i32 1] _ => true | _ => false) &&
            (match drop s4 [.i32 m] with | .ret [] s5 => s5.liveCount == 0 | _ => false)
          | _ => false
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeMapImmKeys = true := by native_decide

/-! #### W5-vec-1: list core (empty/len/get/push/update), immediate-aware. -/

/-- A list of two HEAP elements (box-float): len 2, get reads them back, OOB traps, cascade-drop frees all. -/
private def probeVecHeap : Bool :=
  match boxFloat ({} : HeapState) [.f64 1] with
  | .ret [.i32 f1] s0 =>
    match boxFloat s0 [.f64 2] with
    | .ret [.i32 f2] s1 =>
      match vecEmpty s1 [] with
      | .ret [.i32 ve] s2 =>
        match vecPush s2 [.i32 ve, .i32 f1] with
        | .ret [.i32 v1] s3 =>
          match vecPush s3 [.i32 v1, .i32 f2] with
          | .ret [.i32 v2] s4 =>
            (match vecLen s4 [.i32 v2]          with | .ret [.i32 2] _ => true | _ => false) &&
            (match vecGet s4 [.i32 v2, .i32 0]  with
             | .ret [.i32 g] _ => (match getFloat s4 [.i32 g] with | .ret [.f64 1] _ => true | _ => false)
             | _               => false) &&
            (match vecGet s4 [.i32 v2, .i32 5]  with | .trap _ => true | _ => false) &&
            (s4.liveCount == 3) &&
            (match drop s4 [.i32 v2] with | .ret [] s5 => s5.liveCount == 0 | _ => false)
          | _ => false
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeVecHeap = true := by native_decide

/-- A list of IMMEDIATE int elements: only the vec node is heap (liveCount 1); get yields the immediate,
get-int reads it; cascade-drop skips the immediates and frees just the node → 0. -/
private def probeVecImmElems : Bool :=
  match vecEmpty ({} : HeapState) [] with
  | .ret [.i32 ve] s0 =>
    match boxInt s0 [.i64 5] with
    | .ret [.i32 e] s1 =>
      match vecPush s1 [.i32 ve, .i32 e] with
      | .ret [.i32 v1] s2 =>
        (s2.liveCount == 1) &&
        (match vecGet s2 [.i32 v1, .i32 0] with
         | .ret [.i32 g] _ => (match getInt s2 [.i32 g] with | .ret [.i64 5] _ => true | _ => false)
         | _               => false) &&
        (match drop s2 [.i32 v1] with | .ret [] s3 => s3.liveCount == 0 | _ => false)
      | _ => false
    | _ => false
  | _ => false
example : probeVecImmElems = true := by native_decide

/-- vec-update replaces index 0 (heap float 1 → 3): get 0 reads 3, len still 2, the old element freed so the
census balances to 0 on drop. -/
private def probeVecUpdate : Bool :=
  match boxFloat ({} : HeapState) [.f64 1] with
  | .ret [.i32 f1] s0 =>
    match boxFloat s0 [.f64 2] with
    | .ret [.i32 f2] s1 =>
      match vecEmpty s1 [] with
      | .ret [.i32 ve] s2 =>
        match vecPush s2 [.i32 ve, .i32 f1] with
        | .ret [.i32 v1] s3 =>
          match vecPush s3 [.i32 v1, .i32 f2] with
          | .ret [.i32 v2] s4 =>
            match boxFloat s4 [.f64 3] with
            | .ret [.i32 f3] s5 =>
              match vecUpdate s5 [.i32 v2, .i32 0, .i32 f3] with
              | .ret [.i32 v3] s6 =>
                (match vecLen s6 [.i32 v3]         with | .ret [.i32 2] _ => true | _ => false) &&
                (match vecGet s6 [.i32 v3, .i32 0] with
                 | .ret [.i32 g] _ => (match getFloat s6 [.i32 g] with | .ret [.f64 3] _ => true | _ => false)
                 | _               => false) &&
                (match drop s6 [.i32 v3] with | .ret [] s7 => s7.liveCount == 0 | _ => false)
              | _ => false
            | _ => false
          | _ => false
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeVecUpdate = true := by native_decide

/-! #### Collection CONSUME-op witnesses (map-merge, set-union, set-intersection, set-difference),
immediate-aware. Re-added after the immediates rework (they were dropped from the focused set). Each op
CONSUMES both inputs by dup-and-drop, so the decisive property is the LEAK balance — dropping the result
returns the census to 0 with no orphaned heap object — alongside result value correctness (size + b-wins).
Values are HEAP (box-float, never inlined) so the census tracks them; map keys are IMMEDIATE ints (the
realistic corpus case, and what surfaced the original false-leak diverge). -/

/-- Insert (int key `k`, float-bits value `v`) into map handle `m`; returns (m', s') or none. -/
private def mapInsertIF (s : HeapState) (m : UInt32) (k v : UInt64) : Option (UInt32 × HeapState) :=
  match boxInt s [.i64 k] with
  | .ret [.i32 kh] s1 =>
    match boxFloat s1 [.f64 v] with
    | .ret [.i32 vh] s2 =>
      match mapInsert s2 [.i32 m, .i32 kh, .i32 vh] with
      | .ret [.i32 m'] s3 => some (m', s3)
      | _ => none
    | _ => none
  | _ => none

/-- Build a set of two distinct heap floats {bits `x`, bits `y`}; returns (setHandle, s') or none. -/
private def mkFloatSet2 (s : HeapState) (x y : UInt64) : Option (UInt32 × HeapState) :=
  match boxFloat s [.f64 x] with
  | .ret [.i32 e1] s1 =>
    match boxFloat s1 [.f64 y] with
    | .ret [.i32 e2] s2 =>
      match setEmpty s2 [] with
      | .ret [.i32 se] s3 =>
        match setInsert s3 [.i32 se, .i32 e1] with
        | .ret [.i32 sh1] s4 =>
          match setInsert s4 [.i32 sh1, .i32 e2] with
          | .ret [.i32 sh2] s5 => some (sh2, s5)
          | _ => none
        | _ => none
      | _ => none
    | _ => none
  | _ => none

/-- map-merge: b WINS on the shared key. a={5 to 1.0}, b={5 to 2.0, 6 to 3.0} merges to {5 to 2.0, 6 to 3.0}:
size 2, key 5 reads back b's 2.0, and dropping the merged map returns the census to 0 (a's losing value 1.0
plus both consumed spines freed, no leak). -/
private def probeMapMerge : Bool :=
  match mapEmpty ({} : HeapState) [] with
  | .ret [.i32 ea] s0 =>
    match mapInsertIF s0 ea 5 1 with
    | some (ma, s1) =>
      match mapEmpty s1 [] with
      | .ret [.i32 eb] s2 =>
        match mapInsertIF s2 eb 5 2 with
        | some (mb1, s3) =>
          match mapInsertIF s3 mb1 6 3 with
          | some (mb, s4) =>
            match mapMerge s4 [.i32 ma, .i32 mb] with
            | .ret [.i32 m] s5 =>
              (match mapSize s5 [.i32 m] with | .ret [.i32 2] _ => true | _ => false) &&
              (match boxInt s5 [.i64 5] with
               | .ret [.i32 qk] sq =>
                 (match mapLookup sq [.i32 m, .i32 qk] with
                  | .ret [.i32 g] _ => (match getFloat sq [.i32 g] with | .ret [.f64 2] _ => true | _ => false)
                  | _               => false)
               | _ => false) &&
              (match drop s5 [.i32 m] with | .ret [] s6 => s6.liveCount == 0 | _ => false)
            | _ => false
          | _ => false
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeMapMerge = true := by native_decide

/-- set-union: {1.0,2.0} ∪ {2.0,3.0} = {1.0,2.0,3.0} (dedup drops the incoming duplicate 2.0): size 3, and
dropping the result returns the census to 0 (both consumed sets freed, no leak). -/
private def probeSetUnion : Bool :=
  match mkFloatSet2 ({} : HeapState) 1 2 with
  | some (a, s1) =>
    match mkFloatSet2 s1 2 3 with
    | some (b, s2) =>
      match setUnion s2 [.i32 a, .i32 b] with
      | .ret [.i32 u] s3 =>
        (match setSize s3 [.i32 u] with | .ret [.i32 3] _ => true | _ => false) &&
        (match drop s3 [.i32 u] with | .ret [] s4 => s4.liveCount == 0 | _ => false)
      | _ => false
    | _ => false
  | _ => false
example : probeSetUnion = true := by native_decide

/-- set-intersection: {1.0,2.0} ∩ {2.0,3.0} = {2.0}: size 1, and dropping the result returns the census to 0
(a's non-shared 1.0 plus all of b freed, no leak). -/
private def probeSetIntersection : Bool :=
  match mkFloatSet2 ({} : HeapState) 1 2 with
  | some (a, s1) =>
    match mkFloatSet2 s1 2 3 with
    | some (b, s2) =>
      match setIntersection s2 [.i32 a, .i32 b] with
      | .ret [.i32 r] s3 =>
        (match setSize s3 [.i32 r] with | .ret [.i32 1] _ => true | _ => false) &&
        (match drop s3 [.i32 r] with | .ret [] s4 => s4.liveCount == 0 | _ => false)
      | _ => false
    | _ => false
  | _ => false
example : probeSetIntersection = true := by native_decide

/-- set-difference: {1.0,2.0} minus {2.0,3.0} = {1.0}: size 1, and dropping the result returns the census to 0
(a's shared 2.0 plus all of b freed, no leak). -/
private def probeSetDifference : Bool :=
  match mkFloatSet2 ({} : HeapState) 1 2 with
  | some (a, s1) =>
    match mkFloatSet2 s1 2 3 with
    | some (b, s2) =>
      match setDifference s2 [.i32 a, .i32 b] with
      | .ret [.i32 r] s3 =>
        (match setSize s3 [.i32 r] with | .ret [.i32 1] _ => true | _ => false) &&
        (match drop s3 [.i32 r] with | .ret [] s4 => s4.liveCount == 0 | _ => false)
      | _ => false
    | _ => false
  | _ => false
example : probeSetDifference = true := by native_decide

/-! #### W5.2c: `to-list` witnesses — canonical value-SORTED enumeration + leak balance. Insert keys/elements
OUT of order and assert the list comes back SORTED (the whole point), then drop the list AND the collection
and assert the census returns to 0 (the list co-owns dup'd copies; dropping it + the collection frees all). -/

/-- map-to-list: insert keys 3,1,2 (immediate) with heap-float values 30,10,20; the list is sorted by KEY:
tuple 0 = (1, 10.0), tuple 2 has key 3. Then drop the list + the map → census 0. -/
private def probeMapToList : Bool :=
  match mapEmpty ({} : HeapState) [] with
  | .ret [.i32 e0] s0 =>
    match mapInsertIF s0 e0 3 30 with
    | some (m1, s1) =>
      match mapInsertIF s1 m1 1 10 with
      | some (m2, s2) =>
        match mapInsertIF s2 m2 2 20 with
        | some (m, s3) =>
          match mapToList s3 [.i32 m, .i32 0] with
          | .ret [.i32 lst] s4 =>
            (match vecLen s4 [.i32 lst] with | .ret [.i32 3] _ => true | _ => false) &&
            (match vecGet s4 [.i32 lst, .i32 0] with
             | .ret [.i32 t0] _ =>
               (match arrGet s4 [.i32 t0, .i32 0] with
                | .ret [.i32 k0] _ => (match getInt s4 [.i32 k0] with | .ret [.i64 1] _ => true | _ => false)
                | _ => false) &&
               (match arrGet s4 [.i32 t0, .i32 1] with
                | .ret [.i32 v0] _ => (match getFloat s4 [.i32 v0] with | .ret [.f64 10] _ => true | _ => false)
                | _ => false)
             | _ => false) &&
            (match vecGet s4 [.i32 lst, .i32 2] with
             | .ret [.i32 t2] _ =>
               (match arrGet s4 [.i32 t2, .i32 0] with
                | .ret [.i32 k2] _ => (match getInt s4 [.i32 k2] with | .ret [.i64 3] _ => true | _ => false)
                | _ => false)
             | _ => false) &&
            (match drop s4 [.i32 lst] with
             | .ret [] s5 => (match drop s5 [.i32 m] with | .ret [] s6 => s6.liveCount == 0 | _ => false)
             | _ => false)
          | _ => false
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeMapToList = true := by native_decide

/-- set-to-list: insert 3,1,2 (immediate ints); the list comes back sorted [1,2,3] (elem 0 reads 1, elem 2
reads 3). Then drop the list + the set → census 0 (only the set node was heap; immediates are census-free). -/
private def probeSetToList : Bool :=
  match setEmpty ({} : HeapState) [] with
  | .ret [.i32 se0] s0 =>
    match boxInt s0 [.i64 3] with
    | .ret [.i32 e3] s1 =>
      match setInsert s1 [.i32 se0, .i32 e3] with
      | .ret [.i32 s1h] s2 =>
        match boxInt s2 [.i64 1] with
        | .ret [.i32 e1] s3 =>
          match setInsert s3 [.i32 s1h, .i32 e1] with
          | .ret [.i32 s2h] s4 =>
            match boxInt s4 [.i64 2] with
            | .ret [.i32 e2] s5 =>
              match setInsert s5 [.i32 s2h, .i32 e2] with
              | .ret [.i32 st] s6 =>
                match setToList s6 [.i32 st, .i32 0] with
                | .ret [.i32 lst] s7 =>
                  (match vecLen s7 [.i32 lst] with | .ret [.i32 3] _ => true | _ => false) &&
                  (match vecGet s7 [.i32 lst, .i32 0] with
                   | .ret [.i32 g0] _ => (match getInt s7 [.i32 g0] with | .ret [.i64 1] _ => true | _ => false)
                   | _ => false) &&
                  (match vecGet s7 [.i32 lst, .i32 2] with
                   | .ret [.i32 g2] _ => (match getInt s7 [.i32 g2] with | .ret [.i64 3] _ => true | _ => false)
                   | _ => false) &&
                  (match drop s7 [.i32 lst] with
                   | .ret [] s8 => (match drop s8 [.i32 st] with | .ret [] s9 => s9.liveCount == 0 | _ => false)
                   | _ => false)
                | _ => false
              | _ => false
            | _ => false
          | _ => false
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeSetToList = true := by native_decide

/-! #### W5.3a: bytes + strings witnesses — round-trip, UTF-8 validate/reject, scalar walk, leak balance. -/

/-- bytes round-trip: alloc 3, set bytes "Hi!" (72,105,33), read them back, len 3, OOB traps, drop → 0. -/
private def probeBytesRoundtrip : Bool :=
  match bytesAlloc ({} : HeapState) [.i32 3] with
  | .ret [.i32 b] s0 =>
    match bytesSet s0 [.i32 b, .i32 0, .i32 72] with
    | .ret [.i32 _] s1 =>
      match bytesSet s1 [.i32 b, .i32 1, .i32 105] with
      | .ret [.i32 _] s2 =>
        match bytesSet s2 [.i32 b, .i32 2, .i32 33] with
        | .ret [.i32 _] s3 =>
          (match bytesLen s3 [.i32 b]         with | .ret [.i32 3] _  => true | _ => false) &&
          (match bytesGet s3 [.i32 b, .i32 0] with | .ret [.i32 72] _ => true | _ => false) &&
          (match bytesGet s3 [.i32 b, .i32 2] with | .ret [.i32 33] _ => true | _ => false) &&
          (match bytesGet s3 [.i32 b, .i32 3] with | .trap _ => true | _ => false) &&
          (match drop s3 [.i32 b] with | .ret [] s4 => s4.liveCount == 0 | _ => false)
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeBytesRoundtrip = true := by native_decide

/-- str-from-bytes on VALID UTF-8 ("Hi"): returns the SAME handle (non-NULL, == buf), dropping it → 0. -/
private def probeStrFromBytesValid : Bool :=
  match bytesAlloc ({} : HeapState) [.i32 2] with
  | .ret [.i32 b] s0 =>
    match bytesSet s0 [.i32 b, .i32 0, .i32 72] with
    | .ret [.i32 _] s1 =>
      match bytesSet s1 [.i32 b, .i32 1, .i32 105] with
      | .ret [.i32 _] s2 =>
        match strFromBytes s2 [.i32 b] with
        | .ret [.i32 str] s3 =>
          (str != 0) && (str == b) &&
          (match drop s3 [.i32 str] with | .ret [] s4 => s4.liveCount == 0 | _ => false)
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeStrFromBytesValid = true := by native_decide

/-- str-from-bytes on INVALID UTF-8 (lone 0xFF): returns NULL and CONSUMES (drops) the buffer → census 0. -/
private def probeStrFromBytesInvalid : Bool :=
  match bytesAlloc ({} : HeapState) [.i32 1] with
  | .ret [.i32 b] s0 =>
    match bytesSet s0 [.i32 b, .i32 0, .i32 0xFF] with
    | .ret [.i32 _] s1 =>
      match strFromBytes s1 [.i32 b] with
      | .ret [.i32 str] s2 => (str == 0) && (s2.liveCount == 0)
      | _ => false
    | _ => false
  | _ => false
example : probeStrFromBytesInvalid = true := by native_decide

/-- bytes-scalar-at over a 2-byte UTF-8 scalar "é" (0xC3 0xA9 → U+00E9 = 233): scalar 0 = 233, scalar 1 is
out-of-range → 0xFFFFFFFF; drop → 0. Proves the UTF-8 walk (not a raw byte read). -/
private def probeBytesScalarAt : Bool :=
  match bytesAlloc ({} : HeapState) [.i32 2] with
  | .ret [.i32 b] s0 =>
    match bytesSet s0 [.i32 b, .i32 0, .i32 0xC3] with
    | .ret [.i32 _] s1 =>
      match bytesSet s1 [.i32 b, .i32 1, .i32 0xA9] with
      | .ret [.i32 _] s2 =>
        (match bytesScalarAt s2 [.i32 b, .i32 0] with | .ret [.i32 233] _ => true | _ => false) &&
        (match bytesScalarAt s2 [.i32 b, .i32 1] with | .ret [.i32 0xFFFFFFFF] _ => true | _ => false) &&
        (match drop s2 [.i32 b] with | .ret [] s3 => s3.liveCount == 0 | _ => false)
      | _ => false
    | _ => false
  | _ => false
example : probeBytesScalarAt = true := by native_decide

/-! #### W5.3b: bytes rope witnesses (flat model) — concat / slice (+ OOB trap) / compact, each leak-balanced. -/

/-- bytes-concat: "Hi" (72,105) ++ "!" (33) = "Hi!" — len 3, bytes read back, consumes both operands, drop → 0. -/
private def probeBytesConcat : Bool :=
  match bytesAlloc ({} : HeapState) [.i32 2] with
  | .ret [.i32 a] s0 =>
    match bytesSet s0 [.i32 a, .i32 0, .i32 72] with
    | .ret [.i32 _] s1 =>
      match bytesSet s1 [.i32 a, .i32 1, .i32 105] with
      | .ret [.i32 _] s2 =>
        match bytesAlloc s2 [.i32 1] with
        | .ret [.i32 b] s3 =>
          match bytesSet s3 [.i32 b, .i32 0, .i32 33] with
          | .ret [.i32 _] s4 =>
            match bytesConcat s4 [.i32 a, .i32 b] with
            | .ret [.i32 c] s5 =>
              (match bytesLen s5 [.i32 c]         with | .ret [.i32 3] _  => true | _ => false) &&
              (match bytesGet s5 [.i32 c, .i32 0] with | .ret [.i32 72] _ => true | _ => false) &&
              (match bytesGet s5 [.i32 c, .i32 2] with | .ret [.i32 33] _ => true | _ => false) &&
              (match drop s5 [.i32 c] with | .ret [] s6 => s6.liveCount == 0 | _ => false)
            | _ => false
          | _ => false
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeBytesConcat = true := by native_decide

/-- bytes-slice: "abc" (97,98,99), slice(1,1) = "b" — len 1, byte 98; an OOB slice traps; drop → 0. -/
private def probeBytesSlice : Bool :=
  match bytesAlloc ({} : HeapState) [.i32 3] with
  | .ret [.i32 b] s0 =>
    match bytesSet s0 [.i32 b, .i32 0, .i32 97] with
    | .ret [.i32 _] s1 =>
      match bytesSet s1 [.i32 b, .i32 1, .i32 98] with
      | .ret [.i32 _] s2 =>
        match bytesSet s2 [.i32 b, .i32 2, .i32 99] with
        | .ret [.i32 _] s3 =>
          match bytesSlice s3 [.i32 b, .i32 1, .i32 1] with
          | .ret [.i32 sl] s4 =>
            (match bytesLen s4 [.i32 sl]          with | .ret [.i32 1] _  => true | _ => false) &&
            (match bytesGet s4 [.i32 sl, .i32 0]  with | .ret [.i32 98] _ => true | _ => false) &&
            (match bytesSlice s4 [.i32 sl, .i32 1, .i32 5] with | .trap _ => true | _ => false) &&
            (match drop s4 [.i32 sl] with | .ret [] s5 => s5.liveCount == 0 | _ => false)
          | _ => false
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeBytesSlice = true := by native_decide

/-- bytes-compact: identity in the flat model — same handle, content preserved, drop → 0. -/
private def probeBytesCompact : Bool :=
  match bytesAlloc ({} : HeapState) [.i32 2] with
  | .ret [.i32 b] s0 =>
    match bytesSet s0 [.i32 b, .i32 0, .i32 72] with
    | .ret [.i32 _] s1 =>
      match bytesSet s1 [.i32 b, .i32 1, .i32 105] with
      | .ret [.i32 _] s2 =>
        match bytesCompact s2 [.i32 b] with
        | .ret [.i32 c] s3 =>
          (c == b) &&
          (match bytesLen s3 [.i32 c]         with | .ret [.i32 2] _  => true | _ => false) &&
          (match bytesGet s3 [.i32 c, .i32 1] with | .ret [.i32 105] _ => true | _ => false) &&
          (match drop s3 [.i32 c] with | .ret [] s4 => s4.liveCount == 0 | _ => false)
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeBytesCompact = true := by native_decide

/-! #### Sum (tagged variant) witnesses — heap payload (cascade) + nullary (unit-immediate payload). -/

/-- sum-new(disc 1, heap-float 5): liveCount 2 (sum node + payload); disc reads 1, payload reads back 5;
dropping the sum cascades into the payload → census 0. -/
private def probeSumHeapPayload : Bool :=
  match boxFloat ({} : HeapState) [.f64 5] with
  | .ret [.i32 pl] s0 =>
    match sumNew s0 [.i32 1, .i32 pl] with
    | .ret [.i32 su] s1 =>
      (s1.liveCount == 2) &&
      (match sumDisc s1 [.i32 su] with | .ret [.i32 1] _ => true | _ => false) &&
      (match sumPayload s1 [.i32 su] with
       | .ret [.i32 p] _ => (match getFloat s1 [.i32 p] with | .ret [.f64 5] _ => true | _ => false)
       | _               => false) &&
      (match drop s1 [.i32 su] with | .ret [] s2 => s2.liveCount == 0 | _ => false)
    | _ => false
  | _ => false
example : probeSumHeapPayload = true := by native_decide

/-- A NULLARY variant: sum-new(disc 0, unit-immediate payload from arr-alloc(0)): only the sum node is heap
(liveCount 1); disc 0, payload IS the unit immediate; dropping the sum frees just the node (unit is
census-free) → census 0. -/
private def probeSumNullary : Bool :=
  match arrAlloc ({} : HeapState) [.i32 0] with
  | .ret [.i32 u] s0 =>
    match sumNew s0 [.i32 0, .i32 u] with
    | .ret [.i32 su] s1 =>
      (s1.liveCount == 1) &&
      (match sumDisc s1 [.i32 su]    with | .ret [.i32 0] _ => true | _ => false) &&
      (match sumPayload s1 [.i32 su] with | .ret [.i32 p] _ => p == immUnit | _ => false) &&
      (match drop s1 [.i32 su] with | .ret [] s2 => s2.liveCount == 0 | _ => false)
    | _ => false
  | _ => false
example : probeSumNullary = true := by native_decide

/-! #### List extra-constructor witnesses (concat / prepend / of-arr / drop), each leak-balanced. -/

/-- vec-concat [1] ++ [2] = [1,2] (immediate ints): len 2, elems read 1,2, consumes both inputs, drop → 0. -/
private def probeVecConcat : Bool :=
  match vecEmpty ({} : HeapState) [] with
  | .ret [.i32 e0] s0 =>
    match boxInt s0 [.i64 1] with
    | .ret [.i32 x1] s1 =>
      match vecPush s1 [.i32 e0, .i32 x1] with
      | .ret [.i32 a] s2 =>
        match vecEmpty s2 [] with
        | .ret [.i32 e1] s3 =>
          match boxInt s3 [.i64 2] with
          | .ret [.i32 x2] s4 =>
            match vecPush s4 [.i32 e1, .i32 x2] with
            | .ret [.i32 b] s5 =>
              match vecConcat s5 [.i32 a, .i32 b] with
              | .ret [.i32 c] s6 =>
                (match vecLen s6 [.i32 c] with | .ret [.i32 2] _ => true | _ => false) &&
                (match vecGet s6 [.i32 c, .i32 0] with
                 | .ret [.i32 g] _ => (match getInt s6 [.i32 g] with | .ret [.i64 1] _ => true | _ => false)
                 | _ => false) &&
                (match vecGet s6 [.i32 c, .i32 1] with
                 | .ret [.i32 g] _ => (match getInt s6 [.i32 g] with | .ret [.i64 2] _ => true | _ => false)
                 | _ => false) &&
                (match drop s6 [.i32 c] with | .ret [] s7 => s7.liveCount == 0 | _ => false)
              | _ => false
            | _ => false
          | _ => false
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeVecConcat = true := by native_decide

/-- vec-prepend 1 onto [2] = [1,2] (immediate ints): len 2, elem 0 reads 1, drop → 0. -/
private def probeVecPrepend : Bool :=
  match vecEmpty ({} : HeapState) [] with
  | .ret [.i32 e0] s0 =>
    match boxInt s0 [.i64 2] with
    | .ret [.i32 x2] s1 =>
      match vecPush s1 [.i32 e0, .i32 x2] with
      | .ret [.i32 v] s2 =>
        match boxInt s2 [.i64 1] with
        | .ret [.i32 x1] s3 =>
          match vecPrepend s3 [.i32 v, .i32 x1] with
          | .ret [.i32 p] s4 =>
            (match vecLen s4 [.i32 p] with | .ret [.i32 2] _ => true | _ => false) &&
            (match vecGet s4 [.i32 p, .i32 0] with
             | .ret [.i32 g] _ => (match getInt s4 [.i32 g] with | .ret [.i64 1] _ => true | _ => false)
             | _ => false) &&
            (match drop s4 [.i32 p] with | .ret [] s5 => s5.liveCount == 0 | _ => false)
          | _ => false
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeVecPrepend = true := by native_decide

/-- vec-of-arr: an array [10,20] (immediate ints) becomes a list [10,20]; consumes the array, drop → 0. -/
private def probeVecOfArr : Bool :=
  match arrAlloc ({} : HeapState) [.i32 2] with
  | .ret [.i32 a] s0 =>
    match boxInt s0 [.i64 10] with
    | .ret [.i32 e0] s1 =>
      match arrSet s1 [.i32 a, .i32 0, .i32 e0] with
      | .ret [.i32 _] s2 =>
        match boxInt s2 [.i64 20] with
        | .ret [.i32 e1] s3 =>
          match arrSet s3 [.i32 a, .i32 1, .i32 e1] with
          | .ret [.i32 _] s4 =>
            match vecOfArr s4 [.i32 a] with
            | .ret [.i32 v] s5 =>
              (match vecLen s5 [.i32 v] with | .ret [.i32 2] _ => true | _ => false) &&
              (match vecGet s5 [.i32 v, .i32 1] with
               | .ret [.i32 g] _ => (match getInt s5 [.i32 g] with | .ret [.i64 20] _ => true | _ => false)
               | _ => false) &&
              (match drop s5 [.i32 v] with | .ret [] s6 => s6.liveCount == 0 | _ => false)
            | _ => false
          | _ => false
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeVecOfArr = true := by native_decide

/-- vec-drop: [f1,f2,f3] (HEAP floats) drop prefix [0,1) → tail [f2,f3]; the dropped f1 is FREED by the
consumed list's cascade (liveCount 3 after: tail + f2 + f3); tail elem 0 reads 2.0; drop tail → 0. -/
private def probeVecDrop : Bool :=
  match boxFloat ({} : HeapState) [.f64 1] with
  | .ret [.i32 f1] s0 =>
    match boxFloat s0 [.f64 2] with
    | .ret [.i32 f2] s1 =>
      match boxFloat s1 [.f64 3] with
      | .ret [.i32 f3] s2 =>
        match vecEmpty s2 [] with
        | .ret [.i32 e] s3 =>
          match vecPush s3 [.i32 e, .i32 f1] with
          | .ret [.i32 v1] s4 =>
            match vecPush s4 [.i32 v1, .i32 f2] with
            | .ret [.i32 v2] s5 =>
              match vecPush s5 [.i32 v2, .i32 f3] with
              | .ret [.i32 v3] s6 =>
                match vecDrop s6 [.i32 v3, .i32 1] with
                | .ret [.i32 t] s7 =>
                  (match vecLen s7 [.i32 t] with | .ret [.i32 2] _ => true | _ => false) &&
                  (s7.liveCount == 3) &&
                  (match vecGet s7 [.i32 t, .i32 0] with
                   | .ret [.i32 g] _ => (match getFloat s7 [.i32 g] with | .ret [.f64 2] _ => true | _ => false)
                   | _ => false) &&
                  (match drop s7 [.i32 t] with | .ret [] s8 => s8.liveCount == 0 | _ => false)
                | _ => false
              | _ => false
            | _ => false
          | _ => false
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeVecDrop = true := by native_decide

/-! #### BigInt witnesses — heap leaves (borrow-heavy arith), zero is a HEAP leaf (not null), div/rem trap /0. -/

/-- of-i64 2 + of-i64 3: two heap leaves (liveCount 2); add BORROWS both → fresh sum (liveCount 3); to-i64
reads 5; cmp(2,3) = -1; the caller drops all three (both operands + the owned result) → census 0. -/
private def probeBigIntArith : Bool :=
  match bigintOfI64 ({} : HeapState) [.i64 2] with
  | .ret [.i32 a] s0 =>
    match bigintOfI64 s0 [.i64 3] with
    | .ret [.i32 b] s1 =>
      (s1.liveCount == 2) &&
      (match bigintAdd s1 [.i32 a, .i32 b] with
       | .ret [.i32 c] s2 =>
         (s2.liveCount == 3) &&
         (match bigintToI64Checked s2 [.i32 c] with | .ret [.i64 5] _ => true | _ => false) &&
         (match bigintCmp s2 [.i32 a, .i32 b] with | .ret [.i64 n] _ => n == intToU64Bits (-1) | _ => false) &&
         (match drop s2 [.i32 a] with
          | .ret [] t1 =>
            (match drop t1 [.i32 b] with
             | .ret [] t2 => (match drop t2 [.i32 c] with | .ret [] t3 => t3.liveCount == 0 | _ => false)
             | _ => false)
          | _ => false)
       | _ => false)
    | _ => false
  | _ => false
example : probeBigIntArith = true := by native_decide

/-- Zero is a HEAP leaf, NOT null (construction never canonicalizes): of-i64 0 → a nonzero handle, liveCount 1,
to-i64 reads 0; dividing by it TRAPS; drop → census 0. -/
private def probeBigIntZero : Bool :=
  match bigintOfI64 ({} : HeapState) [.i64 0] with
  | .ret [.i32 z] s0 =>
    (z != 0) && (s0.liveCount == 1) &&
    (match bigintToI64Checked s0 [.i32 z] with | .ret [.i64 0] _ => true | _ => false) &&
    (match bigintOfI64 s0 [.i64 5] with
     | .ret [.i32 a] s1 =>
       (match bigintDiv s1 [.i32 a, .i32 z] with | .trap _ => true | _ => false) &&
       (match drop s1 [.i32 a] with
        | .ret [] t1 => (match drop t1 [.i32 z] with | .ret [] t2 => t2.liveCount == 0 | _ => false)
        | _ => false)
     | _ => false)
  | _ => false
example : probeBigIntZero = true := by native_decide

/-- Truncating div/rem: 7 tdiv 2 = 3, 7 tmod 2 = 1 (both fresh leaves); drop all four → census 0. -/
private def probeBigIntDivRem : Bool :=
  match bigintOfI64 ({} : HeapState) [.i64 7] with
  | .ret [.i32 a] s0 =>
    match bigintOfI64 s0 [.i64 2] with
    | .ret [.i32 b] s1 =>
      match bigintDiv s1 [.i32 a, .i32 b] with
      | .ret [.i32 q] s2 =>
        match bigintRem s2 [.i32 a, .i32 b] with
        | .ret [.i32 r] s3 =>
          (match bigintToI64Checked s3 [.i32 q] with | .ret [.i64 3] _ => true | _ => false) &&
          (match bigintToI64Checked s3 [.i32 r] with | .ret [.i64 1] _ => true | _ => false) &&
          (match drop s3 [.i32 a] with
           | .ret [] t1 =>
             (match drop t1 [.i32 b] with
              | .ret [] t2 =>
                (match drop t2 [.i32 q] with
                 | .ret [] t3 => (match drop t3 [.i32 r] with | .ret [] t4 => t4.liveCount == 0 | _ => false)
                 | _ => false)
              | _ => false)
           | _ => false)
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeBigIntDivRem = true := by native_decide

/-! #### Rational witnesses — normalization (2/4 = 1/2), borrow-heavy arith (1/2 + 1/3 = 5/6), div-by-zero trap. -/

/-- Build a rational `n/d` from two i64 literals (via two BigInt leaves + rational-of); returns (r, s'). -/
private def mkRatI64 (s : HeapState) (n d : UInt64) : Option (UInt32 × HeapState) :=
  match bigintOfI64 s [.i64 n] with
  | .ret [.i32 bn] s1 =>
    match bigintOfI64 s1 [.i64 d] with
    | .ret [.i32 bd] s2 =>
      match rationalOf s2 [.i32 bn, .i32 bd] with
      | .ret [.i32 r] s3 => some (r, s3)
      | _ => none
    | _ => none
  | _ => none

/-- rational-of(2,4) NORMALIZES to 1/2: num reads 1, den reads 2; the rational + its 2 fresh BigInt children =
liveCount 3 (the two input BigInts consumed); dropping num/den handles + the rational balances to census 0. -/
private def probeRationalNormalize : Bool :=
  match bigintOfI64 ({} : HeapState) [.i64 2] with
  | .ret [.i32 b2] s0 =>
    match bigintOfI64 s0 [.i64 4] with
    | .ret [.i32 b4] s1 =>
      match rationalOf s1 [.i32 b2, .i32 b4] with
      | .ret [.i32 r] s2 =>
        (s2.liveCount == 3) &&
        (match rationalNum s2 [.i32 r] with
         | .ret [.i32 nh] s3 =>
           (match bigintToI64Checked s3 [.i32 nh] with | .ret [.i64 1] _ => true | _ => false) &&
           (match drop s3 [.i32 nh] with
            | .ret [] s4 =>
              (match rationalDen s4 [.i32 r] with
               | .ret [.i32 dh] s5 =>
                 (match bigintToI64Checked s5 [.i32 dh] with | .ret [.i64 2] _ => true | _ => false) &&
                 (match drop s5 [.i32 dh] with
                  | .ret [] s6 => (match drop s6 [.i32 r] with | .ret [] s7 => s7.liveCount == 0 | _ => false)
                  | _ => false)
               | _ => false)
            | _ => false)
         | _ => false)
      | _ => false
    | _ => false
  | _ => false
example : probeRationalNormalize = true := by native_decide

/-- 1/2 + 1/3 = 5/6 (numerator reads 5); cmp(1/2, 1/3) = +1; BORROW-heavy (a,b survive the add), so the caller
drops a, b, and the sum → census 0. -/
private def probeRationalArith : Bool :=
  match mkRatI64 ({} : HeapState) 1 2 with
  | some (a, s0) =>
    match mkRatI64 s0 1 3 with
    | some (b, s1) =>
      match rationalAdd s1 [.i32 a, .i32 b] with
      | .ret [.i32 c] s2 =>
        (match rationalNum s2 [.i32 c] with
         | .ret [.i32 nh] s3 =>
           (match bigintToI64Checked s3 [.i32 nh] with | .ret [.i64 5] _ => true | _ => false) &&
           (match drop s3 [.i32 nh] with | .ret [] _ => true | _ => false)
         | _ => false) &&
        (match rationalCmp s2 [.i32 a, .i32 b] with | .ret [.i64 n] _ => n == intToU64Bits 1 | _ => false) &&
        (match drop s2 [.i32 a] with
         | .ret [] t1 =>
           (match drop t1 [.i32 b] with
            | .ret [] t2 => (match drop t2 [.i32 c] with | .ret [] t3 => t3.liveCount == 0 | _ => false)
            | _ => false)
         | _ => false)
      | _ => false
    | _ => false
  | _ => false
example : probeRationalArith = true := by native_decide

/-- rational-div by a zero rational (0/1) TRAPS (the result denominator is 0); drop the operands → census 0. -/
private def probeRationalDivZero : Bool :=
  match mkRatI64 ({} : HeapState) 0 1 with
  | some (z, s0) =>
    match mkRatI64 s0 1 2 with
    | some (a, s1) =>
      (match rationalDiv s1 [.i32 a, .i32 z] with | .trap _ => true | _ => false) &&
      (match drop s1 [.i32 a] with
       | .ret [] t1 => (match drop t1 [.i32 z] with | .ret [] t2 => t2.liveCount == 0 | _ => false)
       | _ => false)
    | _ => false
  | _ => false
example : probeRationalDivZero = true := by native_decide

end Oracle.Heap
