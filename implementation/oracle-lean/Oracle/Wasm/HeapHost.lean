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
cascades a `drop` into each. -/
inductive HeapValue where
  | int     (bits : UInt64)
  | float   (bits : UInt64)
  | float32 (bits : UInt32)
  | bool    (b : Bool)
  | array   (elems : Array UInt32)
deriving Repr, DecidableEq, Inhabited, BEq

/-- The number of owned child handles a value carries (0 for a scalar; the slot count for an array) — the
child set the free-cascade and `mark-immortal-deep` walk. -/
def HeapValue.arity : HeapValue → Nat
  | .array elems => elems.size
  | _            => 0

/-- The owned child handles (array slots; `[]` for a scalar). -/
def HeapValue.children : HeapValue → List UInt32
  | .array elems => elems.toList
  | _            => []

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

end Oracle.Heap
