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
deriving Repr, DecidableEq, Inhabited, BEq

/-- The number of owned child handles a value carries (0 for a scalar; the slot count for an array/map/set) —
the child set the free-cascade and `mark-immortal-deep` walk. -/
def HeapValue.arity : HeapValue → Nat
  | .array e | .map e | .set e => e.size
  | _                          => 0

/-- The owned child handles (array/map/set slots; `[]` for a scalar). -/
def HeapValue.children : HeapValue → List UInt32
  | .array e | .map e | .set e => e.toList
  | _                          => []

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
        | .array e1,  .array e2  =>
          e1.size == e2.size && valueEqWork fuel s (e1.toList.zip e2.toList ++ rest)
        | .map e1,    .map e2    =>
          e1.size == e2.size && valueEqWork fuel s (e1.toList.zip e2.toList ++ rest)
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
    -- immortality
  , ("mark-immortal",      toHostFn [.i32] [.i32]  HeapState.markImmortal)
  , ("mark-immortal-deep", toHostFn [.i32] [.i32]  HeapState.markImmortalDeep) ]

/-! ### Witnesses — compiled every build (a regression fails the oracle-lean build). Immediate-aware after the
IMMEDIATES rework: box-int(fixnum)/box-bool produce inline immediates (no heap, census-excluded, dup/drop
no-op); a value that must be a HEAP object for a refcount/cascade test uses box-float (floats never inline).
This focused set proves the immediates model; the collection consume-op witnesses (map-merge/set-union/vec/…)
are re-added immediate-aware in a follow-up. -/

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

end Oracle.Heap
