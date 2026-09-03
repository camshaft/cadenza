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
deriving Repr, DecidableEq, Inhabited, BEq

/-- The number of owned child handles a value carries (0 for a scalar; the slot count for an array/map) —
the child set the free-cascade and `mark-immortal-deep` walk. -/
def HeapValue.arity : HeapValue → Nat
  | .array e | .map e => e.size
  | _                 => 0

/-- The owned child handles (array/map slots; `[]` for a scalar). -/
def HeapValue.children : HeapValue → List UInt32
  | .array e | .map e => e.toList
  | _                 => []

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

/-- Allocate a fresh live object (rc 1); return its handle + the new state. Handles are **1-based**: `0` is
reserved as the NULL sentinel (an unset array slot / absent handle), so a real handle never collides with
it — the object stored at 0-based index `i` is addressed by handle `i + 1`. -/
def alloc (s : HeapState) (v : HeapValue) : UInt32 × HeapState :=
  ((s.objects.size + 1).toUInt32,
   { s with objects := s.objects.push { value := v, rc := 1, live := true } })

/-- Look up an object by handle (`none` = null handle `0` or a handle never allocated). -/
def getObj? (s : HeapState) (h : UInt32) : Option HeapObject :=
  if h == 0 then none else s.objects[h.toNat - 1]?

/-- Overwrite the object at `h` (caller has already checked `h` is in range via `getObj?`, so `h ≥ 1`). -/
def setObj (s : HeapState) (h : UInt32) (o : HeapObject) : HeapState :=
  { s with objects := s.objects.set! (h.toNat - 1) o }

/-! ### Boxing -/

/-- Box a scalar: allocate + return the handle as an i32. -/
def box (s : HeapState) (v : HeapValue) : HeapResult :=
  let (h, s') := s.alloc v
  .ret [.i32 h] s'

def boxInt : HeapState → List Value → HeapResult
  | s, [.i64 n] => s.box (.int n)
  | s, _        => .trap "box-int: expected (i64)"

def boxFloat : HeapState → List Value → HeapResult
  | s, [.f64 b] => s.box (.float b)
  | s, _        => .trap "box-float: expected (f64)"

def boxFloat32 : HeapState → List Value → HeapResult
  | s, [.f32 b] => s.box (.float32 b)
  | s, _        => .trap "box-float32: expected (f32)"

def boxBool : HeapState → List Value → HeapResult
  | s, [.i32 n] => s.box (.bool (n != 0))
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
  | s, [.i32 h] => s.getWith h "get-int" (fun | .int bits => some (.i64 bits) | _ => none)
  | s, _        => .trap "get-int: expected (i32)"

def getFloat : HeapState → List Value → HeapResult
  | s, [.i32 h] => s.getWith h "get-float" (fun | .float bits => some (.f64 bits) | _ => none)
  | s, _        => .trap "get-float: expected (i32)"

def getFloat32 : HeapState → List Value → HeapResult
  | s, [.i32 h] => s.getWith h "get-float32" (fun | .float32 bits => some (.f32 bits) | _ => none)
  | s, _        => .trap "get-float32: expected (i32)"

def getBool : HeapState → List Value → HeapResult
  | s, [.i32 h] => s.getWith h "get-bool" (fun | .bool b => some (.i32 (if b then 1 else 0)) | _ => none)
  | s, _        => .trap "get-bool: expected (i32)"

/-! ### Arrays (the fixed-arity tuple/record product) -/

/-- `arr-alloc(len) → handle`: a fresh array of `len` NULL (`0`) handle-slots. -/
def arrAlloc : HeapState → List Value → HeapResult
  | s, [.i32 len] => s.box (.array (List.replicate len.toNat (0 : UInt32)).toArray)
  | s, _          => .trap "arr-alloc: expected (i32)"

/-- `arr-set(arr, i, elem) → arr`: store `elem` at slot `i` WITHOUT dup (an ownership MOVE — the array
takes `elem`'s existing reference), returning the array handle for threading. OOB traps. -/
def arrSet : HeapState → List Value → HeapResult
  | s, [.i32 arr, .i32 i, .i32 elem] =>
    match s.getObj? arr with
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
    match s.getObj? arr with
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
    match s.getObj? arr with
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

/-- `dup(h)`: require live (else UAF); on an immortal node it is a NO-OP (sentinel rc); else rc++. -/
def dup : HeapState → List Value → HeapResult
  | s, [.i32 h] =>
    match s.getObj? h with
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
recursively drop the owned children (the cascade). No result. -/
def drop : HeapState → List Value → HeapResult
  | s, [.i32 h] =>
    match s.getObj? h with
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
    match s.getObj? h with
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
    match s.getObj? h with
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
    -- immortality
  , ("mark-immortal",      toHostFn [.i32] [.i32]  HeapState.markImmortal)
  , ("mark-immortal-deep", toHostFn [.i32] [.i32]  HeapState.markImmortalDeep) ]

/-! ### Witnesses — compiled every build, so a regression in the host semantics fails the oracle-lean
build. These exercise the pure `HeapState` layer (no `Store` needed; `Store` has no `Inhabited`). The
`Store`-marshalling `toHostFn` layer + a real emitted heap module are exercised end-to-end by W5.1c's
driver differential. -/

open HeapState

/-- box then get round-trips the value bit-for-bit, and one live object exists. -/
private def probeBoxGet : Bool :=
  match boxInt ({} : HeapState) [.i64 5] with
  | .ret [.i32 h] s1 =>
    (s1.liveCount == 1) &&
    (match getInt s1 [.i32 h] with
     | .ret [.i64 5] _ => true
     | _               => false)
  | _ => false
example : probeBoxGet = true := by native_decide

/-- `dup` raises the refcount; two `drop`s are needed to free (rc 1→2→1→0). liveCount tracks liveness. -/
private def probeDupDrop : Bool :=
  match boxInt ({} : HeapState) [.i64 7] with
  | .ret [.i32 h] s1 =>
    match dup s1 [.i32 h] with
    | .ret [] s2 =>
      (s2.liveCount == 1) &&
      (match drop s2 [.i32 h] with
       | .ret [] s3 =>
         (s3.liveCount == 1) &&
         (match drop s3 [.i32 h] with
          | .ret [] s4 => s4.liveCount == 0
          | _          => false)
       | _ => false)
    | _ => false
  | _ => false
example : probeDupDrop = true := by native_decide

/-- Use-after-free: an access or `drop` of a freed handle traps. -/
private def probeUseAfterFree : Bool :=
  match boxInt ({} : HeapState) [.i64 1] with
  | .ret [.i32 h] s1 =>
    match drop s1 [.i32 h] with
    | .ret [] s2 =>
      (match getInt s2 [.i32 h] with | .trap _ => true | _ => false) &&
      (match drop s2 [.i32 h]   with | .trap _ => true | _ => false)
    | _ => false
  | _ => false
example : probeUseAfterFree = true := by native_decide

/-- Leak oracle: allocating two objects without dropping leaves a non-zero live census. -/
private def probeLeak : Bool :=
  match boxInt ({} : HeapState) [.i64 1] with
  | .ret [.i32 _] s1 =>
    match boxFloat s1 [.f64 0] with
    | .ret [.i32 _] s2 => s2.liveCount == 2
    | _                => false
  | _ => false
example : probeLeak = true := by native_decide

/-- `box-bool` normalizes any non-zero i32 to `true`; `get-bool` returns a canonical 0/1 i32. -/
private def probeBool : Bool :=
  match boxBool ({} : HeapState) [.i32 5] with
  | .ret [.i32 h] s1 =>
    match getBool s1 [.i32 h] with
    | .ret [.i32 1] _ => true
    | _               => false
  | _ => false
example : probeBool = true := by native_decide

/-- A wrong-shape access traps (a boxed float read as an int). -/
private def probeTypeMismatch : Bool :=
  match boxFloat ({} : HeapState) [.f64 0] with
  | .ret [.i32 h] s1 =>
    match getInt s1 [.i32 h] with | .trap _ => true | _ => false
  | _ => false
example : probeTypeMismatch = true := by native_decide

/-! #### W5.1b: arrays, cascade-drop, immortality -/

/-- Build a 2-element array `[box 5, box 9]`, returning the handles for later probes. -/
private def buildPair : Option (UInt32 × UInt32 × UInt32 × HeapState) :=
  match boxInt ({} : HeapState) [.i64 5] with
  | .ret [.i32 h0] s0 =>
    match boxInt s0 [.i64 9] with
    | .ret [.i32 h1] s1 =>
      match arrAlloc s1 [.i32 2] with
      | .ret [.i32 a] s2 =>
        match arrSet s2 [.i32 a, .i32 0, .i32 h0] with
        | .ret [.i32 _] s3 =>
          match arrSet s3 [.i32 a, .i32 1, .i32 h1] with
          | .ret [.i32 _] s4 => some (a, h0, h1, s4)
          | _ => none
        | _ => none
      | _ => none
    | _ => none
  | _ => none

/-- arr-alloc/set/get/len round-trip: `arr-get a 0` returns `h0`, `get-int` reads 5 back, `arr-len` is 2. -/
private def probeArray : Bool :=
  match buildPair with
  | some (a, h0, _, s) =>
    (match arrGet s [.i32 a, .i32 0] with
     | .ret [.i32 g] _ => g == h0
     | _               => false) &&
    (match arrGet s [.i32 a, .i32 0] with
     | .ret [.i32 g] _ =>
       (match getInt s [.i32 g] with | .ret [.i64 5] _ => true | _ => false)
     | _ => false) &&
    (match arrLen s [.i32 a] with | .ret [.i32 2] _ => true | _ => false)
  | none => false
example : probeArray = true := by native_decide

/-- Out-of-bounds `arr-get` / `arr-set` trap. -/
private def probeArrayOob : Bool :=
  match arrAlloc ({} : HeapState) [.i32 1] with
  | .ret [.i32 a] s =>
    (match arrGet s [.i32 a, .i32 5]           with | .trap _ => true | _ => false) &&
    (match arrSet s [.i32 a, .i32 5, .i32 0]   with | .trap _ => true | _ => false)
  | _ => false
example : probeArrayOob = true := by native_decide

/-- Cascade-drop: an array owns its two boxed elements (liveCount 3); dropping the array frees it AND
recursively frees both children → the leak census balances to 0. This is the Perceus dup/drop balance
witness for a compound value. -/
private def probeCascadeDrop : Bool :=
  match buildPair with
  | some (a, _, _, s) =>
    (s.liveCount == 3) &&
    (match drop s [.i32 a] with
     | .ret [] s' => s'.liveCount == 0
     | _          => false)
  | none => false
example : probeCascadeDrop = true := by native_decide

/-- `mark-immortal` excludes a node from the census and makes dup/drop no-ops. -/
private def probeImmortal : Bool :=
  match boxInt ({} : HeapState) [.i64 3] with
  | .ret [.i32 h] s1 =>
    (s1.liveCount == 1) &&
    (match markImmortal s1 [.i32 h] with
     | .ret [.i32 _] s2 =>
       (s2.liveCount == 0) &&
       (match dup s2 [.i32 h] with
        | .ret [] s3 =>
          (match drop s3 [.i32 h] with
           | .ret [] s4 => s4.liveCount == 0
           | _          => false)
        | _ => false)
     | _ => false)
  | _ => false
example : probeImmortal = true := by native_decide

/-- `mark-immortal-deep` marks the array AND its children — the whole structure leaves the census. -/
private def probeImmortalDeep : Bool :=
  match buildPair with
  | some (a, _, _, s) =>
    (s.liveCount == 3) &&
    (match markImmortalDeep s [.i32 a] with
     | .ret [.i32 _] s' => s'.liveCount == 0
     | _                => false)
  | none => false
example : probeImmortalDeep = true := by native_decide

/-- Handles are 1-based: the null sentinel `0` never names a real object, and the first allocation is `1`.
This is load-bearing — the free-cascade / mark-deep skip `h == 0` as a null slot, so a 0-based handle would
collide with the sentinel and silently skip dropping/marking the first-allocated object. -/
private def probeNullSentinel : Bool :=
  match boxInt ({} : HeapState) [.i64 0] with
  | .ret [.i32 h] s => (h == 1) && (s.getObj? 0).isNone && (s.getObj? 1).isSome
  | _               => false
example : probeNullSentinel = true := by native_decide

/-! #### W5.1b coverage hardening — deeper cascade / sharing / DAG invariants (pinning edges the base
witnesses do not exercise). -/

/-- Nested-array cascade: array A holds array B holds a boxed int (3 live objects). Dropping A frees
A → B → the int transitively — the MULTI-LEVEL cascade + traversal-fuel depth. Leak census → 0. -/
private def probeNestedCascade : Bool :=
  match boxInt ({} : HeapState) [.i64 7] with
  | .ret [.i32 h] s0 =>
    match arrAlloc s0 [.i32 1] with
    | .ret [.i32 b] s1 =>
      match arrSet s1 [.i32 b, .i32 0, .i32 h] with
      | .ret [.i32 _] s2 =>
        match arrAlloc s2 [.i32 1] with
        | .ret [.i32 a] s3 =>
          match arrSet s3 [.i32 a, .i32 0, .i32 b] with
          | .ret [.i32 _] s4 =>
            (s4.liveCount == 3) &&
            (match drop s4 [.i32 a] with
             | .ret [] s5 => s5.liveCount == 0
             | _          => false)
          | _ => false
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeNestedCascade = true := by native_decide

/-- Shared child: an array owns ONE ref to a child that is ALSO held elsewhere (rc 2). Dropping the array
decrements the child but does NOT free it (rc 2→1, still live); dropping the remaining ref then frees it.
Pins the sharing refcount discipline (a single owner's drop must not free a shared node). -/
private def probeSharedChild : Bool :=
  match boxInt ({} : HeapState) [.i64 1] with
  | .ret [.i32 h] s0 =>
    match dup s0 [.i32 h] with
    | .ret [] s1 =>
      match arrAlloc s1 [.i32 1] with
      | .ret [.i32 a] s2 =>
        match arrSet s2 [.i32 a, .i32 0, .i32 h] with
        | .ret [.i32 _] s3 =>
          (s3.liveCount == 2) &&
          (match drop s3 [.i32 a] with
           | .ret [] s4 =>
             (s4.liveCount == 1) &&
             (match drop s4 [.i32 h] with
              | .ret [] s5 => s5.liveCount == 0
              | _          => false)
           | _ => false)
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeSharedChild = true := by native_decide

/-- DAG-safe mark-immortal-deep: an array with TWO slots pointing to the SAME child. The deep-mark visits
the shared child ONCE (the already-immortal skip), marking array + child immortal → census 0, no
double-processing of the shared node. -/
private def probeDagMarkDeep : Bool :=
  match boxInt ({} : HeapState) [.i64 2] with
  | .ret [.i32 h] s0 =>
    match dup s0 [.i32 h] with
    | .ret [] s1 =>
      match arrAlloc s1 [.i32 2] with
      | .ret [.i32 a] s2 =>
        match arrSet s2 [.i32 a, .i32 0, .i32 h] with
        | .ret [.i32 _] s3 =>
          match arrSet s3 [.i32 a, .i32 1, .i32 h] with
          | .ret [.i32 _] s4 =>
            (s4.liveCount == 2) &&
            (match markImmortalDeep s4 [.i32 a] with
             | .ret [.i32 _] s5 => s5.liveCount == 0
             | _                => false)
          | _ => false
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeDagMarkDeep = true := by native_decide

/-! #### W5.2a: positional maps (map-alloc/set/key/val/len). -/

/-- Build a 1-entry map {box 5 ↦ box 9}: box the key + value, map-alloc 1, map-set pair 0. -/
private def buildMap1 : Option (UInt32 × UInt32 × UInt32 × HeapState) :=
  match boxInt ({} : HeapState) [.i64 5] with
  | .ret [.i32 k] s0 =>
    match boxInt s0 [.i64 9] with
    | .ret [.i32 v] s1 =>
      match mapAlloc s1 [.i32 1] with
      | .ret [.i32 m] s2 =>
        match mapSet s2 [.i32 m, .i32 0, .i32 k, .i32 v] with
        | .ret [.i32 _] s3 => some (m, k, v, s3)
        | _ => none
      | _ => none
    | _ => none
  | _ => none

/-- map-alloc/set/key/val/len round-trip: map-key/val at pair 0 return the boxed key/value; map-len is 1. -/
private def probeMap : Bool :=
  match buildMap1 with
  | some (m, k, v, s) =>
    (match mapKey s [.i32 m, .i32 0] with | .ret [.i32 g] _ => g == k | _ => false) &&
    (match mapVal s [.i32 m, .i32 0] with | .ret [.i32 g] _ => g == v | _ => false) &&
    (match mapLen s [.i32 m]         with | .ret [.i32 1] _ => true   | _ => false)
  | none => false
example : probeMap = true := by native_decide

/-- Out-of-bounds map-key / map-set trap. -/
private def probeMapOob : Bool :=
  match mapAlloc ({} : HeapState) [.i32 1] with
  | .ret [.i32 m] s =>
    (match mapKey s [.i32 m, .i32 5]                 with | .trap _ => true | _ => false) &&
    (match mapSet s [.i32 m, .i32 5, .i32 0, .i32 0] with | .trap _ => true | _ => false)
  | _ => false
example : probeMapOob = true := by native_decide

/-- Cascade-drop a map frees it AND both its key + value handles (leak census → 0). -/
private def probeMapCascade : Bool :=
  match buildMap1 with
  | some (m, _, _, s) =>
    (s.liveCount == 3) &&
    (match drop s [.i32 m] with
     | .ret [] s' => s'.liveCount == 0
     | _          => false)
  | none => false
example : probeMapCascade = true := by native_decide

/-! #### W5.2b-1: functional map read-only (map-empty/lookup/size) + structural value-eq key match. -/

/-- map-lookup matches by structural VALUE-equality, not handle identity: a SECOND key handle boxed with the
same value (5) as the stored key still finds the entry's value. -/
private def probeMapLookup : Bool :=
  match buildMap1 with
  | some (m, k, v, s) =>
    match boxInt s [.i64 5] with
    | .ret [.i32 k2] s2 =>
      (k2 != k) &&
      (match mapLookup s2 [.i32 m, .i32 k2] with
       | .ret [.i32 got] _ => got == v
       | _                 => false)
    | _ => false
  | none => false
example : probeMapLookup = true := by native_decide

/-- map-lookup of an absent key returns NULL (0). -/
private def probeMapLookupMiss : Bool :=
  match buildMap1 with
  | some (m, _, _, s) =>
    match boxInt s [.i64 7] with
    | .ret [.i32 k7] s2 =>
      (match mapLookup s2 [.i32 m, .i32 k7] with
       | .ret [.i32 0] _ => true
       | _               => false)
    | _ => false
  | none => false
example : probeMapLookupMiss = true := by native_decide

/-- map-empty is size 0; a 1-entry map is size 1. -/
private def probeMapEmptySize : Bool :=
  match mapEmpty ({} : HeapState) [] with
  | .ret [.i32 e] s0 =>
    (match mapSize s0 [.i32 e] with | .ret [.i32 0] _ => true | _ => false) &&
    (match buildMap1 with
     | some (m, _, _, s) => (match mapSize s [.i32 m] with | .ret [.i32 1] _ => true | _ => false)
     | none              => false)
  | _ => false
example : probeMapEmptySize = true := by native_decide

/-! #### W5.2b-2: functional map CONSUMING ops (map-insert/map-remove) — value + leak balance. -/

/-- Insert a NEW key into the empty map: lookup finds it, size 1, and dropping the result frees everything
(leak census → 0). -/
private def probeMapInsertNew : Bool :=
  match boxInt ({} : HeapState) [.i64 5] with
  | .ret [.i32 k] s0 =>
    match boxInt s0 [.i64 9] with
    | .ret [.i32 v] s1 =>
      match mapEmpty s1 [] with
      | .ret [.i32 e] s2 =>
        match mapInsert s2 [.i32 e, .i32 k, .i32 v] with
        | .ret [.i32 m] s3 =>
          (match mapLookup s3 [.i32 m, .i32 k] with | .ret [.i32 g] _ => g == v | _ => false) &&
          (match mapSize s3 [.i32 m]         with | .ret [.i32 1] _ => true   | _ => false) &&
          (match drop s3 [.i32 m]            with | .ret [] s4 => s4.liveCount == 0 | _ => false)
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeMapInsertNew = true := by native_decide

/-- Insert an EXISTING key (matched by value-eq via a distinct key handle) REPLACES the value: lookup returns
the new value, and dropping the result frees everything (→ 0) — proving the old value + redundant incoming
key were dropped (else the census would not balance). -/
private def probeMapInsertReplace : Bool :=
  match boxInt ({} : HeapState) [.i64 5] with
  | .ret [.i32 k] s0 =>
    match boxInt s0 [.i64 90] with
    | .ret [.i32 v0] s1 =>
      match mapEmpty s1 [] with
      | .ret [.i32 e] s2 =>
        match mapInsert s2 [.i32 e, .i32 k, .i32 v0] with
        | .ret [.i32 m0] s3 =>
          match boxInt s3 [.i64 5] with
          | .ret [.i32 k2] s4 =>
            match boxInt s4 [.i64 91] with
            | .ret [.i32 v1] s5 =>
              match mapInsert s5 [.i32 m0, .i32 k2, .i32 v1] with
              | .ret [.i32 m1] s6 =>
                (match mapLookup s6 [.i32 m1, .i32 k] with | .ret [.i32 g] _ => g == v1 | _ => false) &&
                (match drop s6 [.i32 m1] with | .ret [] s7 => s7.liveCount == 0 | _ => false)
              | _ => false
            | _ => false
          | _ => false
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeMapInsertReplace = true := by native_decide

/-- Remove an existing key (matched by value-eq) → empty map; the removed key+value are freed (borrowed query
survives). Dropping the result + the borrowed query → 0. -/
private def probeMapRemove : Bool :=
  match boxInt ({} : HeapState) [.i64 5] with
  | .ret [.i32 k] s0 =>
    match boxInt s0 [.i64 9] with
    | .ret [.i32 v] s1 =>
      match mapEmpty s1 [] with
      | .ret [.i32 e] s2 =>
        match mapInsert s2 [.i32 e, .i32 k, .i32 v] with
        | .ret [.i32 m] s3 =>
          match boxInt s3 [.i64 5] with
          | .ret [.i32 k2] s4 =>
            match mapRemove s4 [.i32 m, .i32 k2] with
            | .ret [.i32 m2] s5 =>
              (match mapSize s5 [.i32 m2]              with | .ret [.i32 0] _ => true | _ => false) &&
              (match mapLookup s5 [.i32 m2, .i32 k2]   with | .ret [.i32 0] _ => true | _ => false) &&
              (match drop s5 [.i32 m2] with
               | .ret [] s6 => (match drop s6 [.i32 k2] with | .ret [] s7 => s7.liveCount == 0 | _ => false)
               | _          => false)
            | _ => false
          | _ => false
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeMapRemove = true := by native_decide

/-- Remove of an ABSENT key is a no-op identity: the same handle back, size unchanged. -/
private def probeMapRemoveAbsent : Bool :=
  match buildMap1 with
  | some (m, _, _, s) =>
    match boxInt s [.i64 7] with
    | .ret [.i32 k7] s2 =>
      match mapRemove s2 [.i32 m, .i32 k7] with
      | .ret [.i32 m2] s3 =>
        (m2 == m) && (match mapSize s3 [.i32 m2] with | .ret [.i32 1] _ => true | _ => false)
      | _ => false
    | _ => false
  | none => false
example : probeMapRemoveAbsent = true := by native_decide

/-- SHARED-map insert path-copies (the case v-lean-oracle flagged): inserting into a map with rc>1 leaves the
ORIGINAL unchanged and produces a SEPARATE updated version — both coexist, then both drop to 0 with no leak.
This exercises the dup-and-drop transfer's shared branch (drop m does NOT cascade; result holds dup'd refs). -/
private def probeMapInsertShared : Bool :=
  match boxInt ({} : HeapState) [.i64 5] with
  | .ret [.i32 k] s0 =>
    match boxInt s0 [.i64 90] with
    | .ret [.i32 v0] s1 =>
      match mapEmpty s1 [] with
      | .ret [.i32 e] s2 =>
        match mapInsert s2 [.i32 e, .i32 k, .i32 v0] with
        | .ret [.i32 m0] s3 =>
          match dup s3 [.i32 m0] with
          | .ret [] s4 =>
            match boxInt s4 [.i64 5] with
            | .ret [.i32 k2] s5 =>
              match boxInt s5 [.i64 91] with
              | .ret [.i32 v1] s6 =>
                match mapInsert s6 [.i32 m0, .i32 k2, .i32 v1] with
                | .ret [.i32 m1] s7 =>
                  (match mapLookup s7 [.i32 m0, .i32 k] with | .ret [.i32 g] _ => g == v0 | _ => false) &&
                  (match mapLookup s7 [.i32 m1, .i32 k] with | .ret [.i32 g] _ => g == v1 | _ => false) &&
                  (match drop s7 [.i32 m0] with
                   | .ret [] s8 => (match drop s8 [.i32 m1] with | .ret [] s9 => s9.liveCount == 0 | _ => false)
                   | _          => false)
                | _ => false
              | _ => false
            | _ => false
          | _ => false
        | _ => false
      | _ => false
    | _ => false
  | _ => false
example : probeMapInsertShared = true := by native_decide

end Oracle.Heap
