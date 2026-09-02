/-
A clean-room Lean host-function model of the cdz-runtime `"heap"` import surface — the W5.1a slice.

The emitted core module imports the value-heap runtime ops (module name `"heap"`, kebab-case op names)
that talos declines today because it runs with an empty host. This file models those ops as pure Lean
state transformers over a `HeapState`, so `talosDriver` can supply them as `HostFn`s (W5.1c) and the
heap/collection corpus cases become runnable instead of skipped.

W5.1a subset — the ops with unambiguous, spec-clear semantics:
  * refcount / liveness core: `dup`, `drop`, `live-objects`
  * boxing:                   box-{int,float,float32,bool} + get-{int,float,float32,bool}
(`mark-immortal*`, `reset`, and the `arr-*` family land in W5.1b once their exact return semantics are
confirmed from the spec; adding them here without confirming would be guessing.)

INDEPENDENCE (the whole point of the differential): the semantics are modeled from the SPEC's observable
value form + refcount discipline, NOT by linking the real cdz-runtime wasm — else a runtime bug would hide
on both sides. The state is refcount + liveness aware from the start (the operator's Perceus constraint):
an access to a freed handle traps (use-after-free); a `drop` of a freed / rc-0 handle traps (double-free);
`live-objects` counts non-immortal live objects, so a leak is a non-zero count at end of run.

Imports ONLY talos's `Syntax` (`Value`/`ValueType`/`Store`) + `Host` (`HostFn`/`HostResult`) — both are
Std-only, so this stays Mathlib-free like the rest of the execution path.
-/
import Interpreter.Wasm.Syntax
import Interpreter.Wasm.Host

open _root_.Wasm (Value ValueType Store HostFn HostResult)

namespace Oracle.Heap

/-- A boxed runtime scalar, stored as the exact wasm payload it was boxed from so `get` round-trips `box`
bit-for-bit. `bool` normalizes (any non-zero i32 boxes to `true`), matching the runtime's boolean box. -/
inductive HeapValue where
  | int     (bits : UInt64)
  | float   (bits : UInt64)
  | float32 (bits : UInt32)
  | bool    (b : Bool)
deriving Repr, DecidableEq, Inhabited, BEq

/-- One heap object: its value, refcount, liveness (`false` once freed at rc 0), and immortality flag
(immortal objects are excluded from the leak census). -/
structure HeapObject where
  value    : HeapValue
  rc       : Nat
  live     : Bool
  immortal : Bool := false
deriving Repr, Inhabited

/-- The host state `α`: a growable pool of heap objects addressed by `u32` handle (= pool index). Fresh
allocations append, so a handle is stable for the run (a freed slot is marked, never reused in W5.1a — the
reuse specialization is W5.4). -/
structure HeapState where
  objects : Array HeapObject := #[]
deriving Repr, Inhabited

/-- The outcome of a heap op: either result values + the new state, or a trap (a UAF / double-free / bad
argument). Mapped to talos's `HostResult` by `toHostFn`. -/
inductive HeapResult where
  | ret  (vals : List Value) (s : HeapState)
  | trap (msg : String)
deriving Repr

namespace HeapState

/-- The leak oracle: the number of live, non-immortal objects. Must be 0 at end of run, else a leak. -/
def liveCount (s : HeapState) : Nat :=
  s.objects.foldl (fun n o => if o.live && !o.immortal then n + 1 else n) 0

/-- Allocate a fresh live object (rc 1); return its handle + the new state. -/
def alloc (s : HeapState) (v : HeapValue) : UInt32 × HeapState :=
  (s.objects.size.toUInt32,
   { s with objects := s.objects.push { value := v, rc := 1, live := true } })

/-- Look up an object by handle (`none` = handle never allocated). -/
def getObj? (s : HeapState) (h : UInt32) : Option HeapObject := s.objects[h.toNat]?

/-- Overwrite the object at `h` (caller has already checked `h` is in range via `getObj?`). -/
def setObj (s : HeapState) (h : UInt32) (o : HeapObject) : HeapState :=
  { s with objects := s.objects.set! h.toNat o }

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

/-! ### Refcount / liveness core -/

/-- `dup(h)`: require live (else UAF), rc++. No result. -/
def dup : HeapState → List Value → HeapResult
  | s, [.i32 h] =>
    match s.getObj? h with
    | none   => .trap s!"dup: unknown handle {h}"
    | some o =>
      if !o.live then .trap s!"dup: use-after-free (handle {h} freed)"
      else .ret [] (s.setObj h { o with rc := o.rc + 1 })
  | s, _ => .trap "dup: expected (i32)"

/-- `drop(h)`: require live + rc>0 (else double-free), rc--; at 0 → freed. No result. (Boxed scalars have
no child handles, so there is nothing to recursively drop in W5.1a — recursive child-drop lands with the
container ops in W5.1b/W5.4.) -/
def drop : HeapState → List Value → HeapResult
  | s, [.i32 h] =>
    match s.getObj? h with
    | none   => .trap s!"drop: unknown handle {h}"
    | some o =>
      if !o.live then .trap s!"drop: double-free (handle {h} already freed)"
      else if o.rc == 0 then .trap s!"drop: double-free (handle {h} refcount already zero)"
      else
        let rc' := o.rc - 1
        .ret [] (s.setObj h { o with rc := rc', live := rc' != 0 })
  | s, _ => .trap "drop: expected (i32)"

/-- `live-objects()`: the live non-immortal census (the leak oracle), as an i32. -/
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

/-- The W5.1a `"heap"` ops, keyed by their exact emitted kebab-case name + core signature (from
`rcdzc/src/backend/wasm/runtime_abi.rs`). W5.1c builds a `HostRegistry` from this by pairing each with an
`ImportDecl { «module» := "heap", name, params, results }`; later increments extend the table. -/
def w51aHeapOps : List (String × HostFn HeapState) :=
  [ ("dup",          toHostFn [.i32] []      HeapState.dup)
  , ("drop",         toHostFn [.i32] []      HeapState.drop)
  , ("live-objects", toHostFn []     [.i32]  HeapState.liveObjects)
  , ("box-int",      toHostFn [.i64] [.i32]  HeapState.boxInt)
  , ("box-float",    toHostFn [.f64] [.i32]  HeapState.boxFloat)
  , ("box-float32",  toHostFn [.f32] [.i32]  HeapState.boxFloat32)
  , ("box-bool",     toHostFn [.i32] [.i32]  HeapState.boxBool)
  , ("get-int",      toHostFn [.i32] [.i64]  HeapState.getInt)
  , ("get-float",    toHostFn [.i32] [.f64]  HeapState.getFloat)
  , ("get-float32",  toHostFn [.i32] [.f32]  HeapState.getFloat32)
  , ("get-bool",     toHostFn [.i32] [.i32]  HeapState.getBool) ]

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

end Oracle.Heap
