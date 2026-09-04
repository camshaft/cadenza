# The wasm-oracle — the wasm half of the differential oracle

`Oracle.Wasm` is the **wasm-interpreter half** of the Lean differential oracle (vertical
`v-wasm-oracle`). It pairs with the Core/`denote` half (owned by `v-lean-oracle`): the operator's goal is
to assert compilation is correct **all the way through** — source → Core → *emitted wasm* — not just to
the Core layer, via static/symbolic program-equivalence.

- **This half** parses the per-case emitted wasm, runs it, and reports an `Oracle.Outcome`.
- **The differential glue** (`v-lean-oracle`) asserts `Core denote(P) == run_wasm(compile(P))` for all
  inputs — ideally one Lean theorem, built on the wasm interpreter's total-correctness WP layer.

See also: `PRINCIPLES.md` (clean-room, corpus-conformance, the two stages) and `FRAME.md` (the
`cdz-oracle` wire frame). Design: `implementation/design/DESIGN-lean-differential-oracle.md`.

## The interpreter: talos (pinned + integrated, driving the daily full-corpus differential)

Per an operator-cleared decision (2026-08-31), the wasm interpreter is **`cajal-technologies/talos`** — a
Lean 4 wasm interpreter (AGPL; fine — the oracle is an internal, non-distributed verification tool, not
linked into the shipped compiler) whose execution semantics and total-correctness **WP proof layer** live
in one codebase (highest confidence), matching the "do two programs do the same thing" goal. It is pinned
as a nix flake input (`fetchFromGitHub` + hash) — hermetic. talos requires **Lean 4.32.2** (oracle-lean was
4.30.0) and pulls **full Mathlib**; the toolchain bump + Mathlib-in-nix are co-owned with `v-nix` and land
as an isolated, opt-in check (never the fleet default gate). talos ingests `.wat` and interprets **core**
wasm (its runner pre-flight REJECTS a module with imports).

## The emitted-artifact anatomy (what we parse)

`cdz compile case.sexp --target wasm` emits a **WebAssembly COMPONENT**, not a bare core module. For a
scalar case `(do (def (main) 5) (export main))`:

- a **core module** with the real computation (`main : () -> i64`, `i64.const 5`);
- a component envelope: `(core instance (instantiate 0))`, a `(canon lift)` (i64 → s64), `(export "main")`;
- a `@custom "cdz-result-type"` section = a binary-AST module `(result-types (result-type <entry>
  <TypeName>) …)` giving the entry's Cadenza result type (scalar spellings seen: `Int`, `Bool`, `Float`).

The scalar/arith subset unbundles to a **zero-import** core module (talos-runnable). Heap/collection cases
unbundle to multiple core modules, one importing the cdz-runtime (`"heap" …`) → talos rejects it → we report
`.unsupported` (a sound, skipped coverage gap) until the runtime imports are modeled (a later increment).

## The `run_wasm` pipeline

```
emitted COMPONENT .wasm
  │  (harness, IMPURE — outside pure Lean)
  │  wasm-tools component unbundle --threshold 0   → the embedded core module  (--threshold 0 REQUIRED)
  │  wasm-tools print                              → core-module .wat text
  ▼
core-module .wat text  +  cdz-result-type section bytes
  │  (talos, PURE — Wasm.Decoder.Wat.decode + Wasm.SmallStep, driven in-process so run_wasm is a
  │   provable Lean term for the differential theorem — NOT a shelled exe)
  ▼
a raw wasm run result (WasmOutcome / WasmVal)
  │  (Oracle.Wasm, PURE)
  │  resultScalarTy? (decode cdz-result-type → ScalarTy)   +   toOutcome (map raw result → Value/Outcome)
  ▼
Oracle.Outcome     (consumed by v-lean-oracle's differential glue)
```

The talos exit-code contract maps directly onto `Oracle.Outcome`: `OK`→`.value`, `TRAP`→`.trap`,
`OUT_OF_FUEL`→`.diverges`, `ERR` (imports present / decode-fail / bad method)→`.unsupported`.

### The `Driver` adapter (what the talos-drive slice writes)

The `Driver := (coreWat : String) → Trial → WasmOutcome` seam (see `Oracle.Wasm`) is filled by an adapter
importing **only** `Interpreter.Wasm.SmallStep` + `Interpreter.Wasm.Decoder.Wat` (their closure is Std-only,
Mathlib-free — see below). It replicates talos's own runner invocation (`α := Unit` ⇒ empty host, matching
the self-contained subset):

```
let m ← Wasm.Decoder.Wat.decode wat                 -- Except _ Module          (.error → .err → .unsupported)
let idx ← m.findExport entry                        -- (none → unknown export → .err)
let store0 := m.runConstGlobals fuel (m.initialStore (α := Unit)) {}
let store0 := m.runActiveSegments fuel store0 {}
let inst : Wasm.SmallStep.ModuleInstance Unit := { module := m, host := {} }
let cfg  ← Wasm.SmallStep.initConfig inst idx store0 vs.reverse    -- params in STACK order (reversed)
match (Wasm.SmallStep.runSteps fuel cfg).result with
| .success results _   => .ok (results.reverse.map talosValToWasmVal)
| .trapped reason _    => .trap reason.message         -- (+ special .uncaughtException case)
| .outOfFuel _         => .outOfFuel
| .internalError err _ => .err err.message
```

`talosVal → WasmVal`: `i32 v → .i32 v.toInt32.toInt` (SIGNED), `i64 → .i64 v.toInt64.toInt`, `f32 b → .f32 b`,
`f64 b → .f64 b`.

## Interface with v-lean-oracle (confirmed)

- Reuse the shared `Oracle.Value` + `Oracle.Eval.Outcome` — no parallel model.
- `Oracle.Wasm` exposes `runWasm (core-module-or-component bytes) (trial) -> Outcome`, owning parse +
  unbundle + talos drive + result decode. The talos-value → `Oracle.Value` boundary mapping is co-owned.
- Scalar width/signedness discipline: wasm `i32`/`i64` are two's-complement; the interpreter yields the
  signed `Int`, and Cadenza `.int` is arbitrary-precision, so a scalar-int result maps to `.int` at the
  ascribed width (same discipline as `evalArithOp`); `f32`/`f64` → `.f64` (Float32 rounding).

## Increment ladder

- **W1 — boundary mapping** ✅ (`toOutcome`, PR #7177): interpreter-agnostic `WasmVal`/`ScalarTy`/
  `WasmOutcome` + the total exit-code/scalar → `Oracle.Outcome` map + witnesses. Builds on 4.30, no talos dep.
- **W1.1 — result-type resolver** ✅ (`resultScalarTy?`, PR #7182): decode the `cdz-result-type` section
  (via `Oracle.Ast`) → `ScalarTy`; verified scalar spellings mapped, unknown → `.unsupported`. No talos dep.
- **W0 — toolchain + talos pin** ✅ (co-owned w/ v-nix, PR #7294): `lean-toolchain` → v4.32.2, talos flake
  input + lakefile `require` + all-`path` lake-manifest (offline, Mathlib-free execution closure), oracleLean
  on `lean4_432` + autoPatchelf — landed atomically.
- **W2/W3 — talos drive** ✅ (`talosDriver`, PR #7294): `runWasmWith` drives talos (`Wasm.Decoder.Wat.decode`
  + `Wasm.SmallStep.runSteps`) on the extracted core module → `WasmOutcome` → `toOutcome`. native_decide
  witnesses prove talos runs (`main()=5` → `.ok #[.i64 5]`).
- **W4 — self-contained scalar/arith subset** ✅ (FIRST FULL-CORPUS RUN, daily): emit-extraction harness
  (`oracle-wasm-case-dirs`, uncapped Step A over 01-literals + 06-numeric-model, PR #7630) + both cachix warm
  layers (emit #7427/#7546, extraction #7633/#7636) + the daily `oracle-wasm-diff` check wired into
  `cache-warm-extract-wasm.yml` (#7698). First run: **216 AGREE / 1 diverge / 927 skip** of 1144 — the 1
  diverge (`06-numeric-model-1398`) was triaged to a RESOLVER FALSE-POSITIVE (see the lesson below), fixed by
  #7715, so **0 real miscompiles at Step-A scale**. Active follow-ups: widen `scalarTyOfName?` from the
  skip-reason head histogram (head-tag #7708 + histogram #7725), model non-nullary entries (the
  `local.get index 0 invalid` / arity-mismatch skip cluster), then **Step B** = widen `wasmOracleFiles` to the
  whole corpus.

> **Lesson — a multi-value/tuple result-type must resolve to `none` (SKIP), never truncate to a scalar.** A
> tuple `main` emits the FLAT `cdz-result-type` `(result-type "main" (Int 64) (Int 64) (Int 64) (Int 64))` —
> multiple type children. The original `cs.size ≥ 3` guard accepted it and read only `cs[2]` (the first
> element), leaking a compound result through `resultScalarTy?` as `.int` → a false DIVERGE (Core ref = a
> tuple, wasm = one Int). Fix (#7715): require EXACTLY one type child (`cs.size == 3`) in
> `resultScalarTyOfModule?` + `unmodeledResultHead?`. Compound returns skip soundly until W5 models them.
- **W5+ — runtime-importing cases**: heap/collection cases import the cdz-runtime (`"heap" …`); satisfy those
  imports with clean-room Lean host functions (see "Running imported runtime functions" below), with the host
  state refcount-and-liveness-aware from the start. The deep end; scalars first — but FEASIBLE, not a ceiling.
- **W6 — Perceus soundness (dynamic)**: the rc+liveness heap host makes every heap-case run also witness
  no-UAF / no-double-free / no-leak (trap on freed access/double-drop; assert empty live-set at end). Nearly
  free given the W5 host; an independent mirror of the debug-counters runtime.
- **W7 — Perceus soundness (symbolic, aspirational capstone)**: prove no-UAF/no-double-free/no-leak FOR ALL
  INPUTS via talos's WP + a `HostContract` refcount invariant. Theorem-shaped → co-owned w/ v-lean-oracle.

## Running imported runtime functions — the host API (W5+ feasibility)

talos's interpreter is **host-parameterized** (`Store α` carries a `host : α` the wasm core never inspects);
the runner declines imports only because it picks the trivial `α := Unit`/empty host. The library supports
real imports first-class:

- **`HostFn α = { params, results, invoke : Store α → List Value → HostResult α }`** — the import's behavior
  as a Lean function; `invoke` may read/write the store (incl. linear **memory**) and thread host state `α`.
  `HostResult α = .Return vals store' | .Trap store' msg | .Throw …`.
- **`HostEnv α = { funcs : List (HostFn α) }`** — positional, indexed like the module's `imports`
  (`call i` → `funcs[i]`). A name-keyed **`HostRegistry`** builds it per-module by walking `m.imports`
  (unresolved → trapping stub; total). `Host.Universal` composes several hosts via `HostLens`.
- **Proof side:** `HostSpec`/`HostContract` let a program be verified *parametric over any host satisfying a
  contract* (CompCert/seL4 "abstract oracle" pattern) — so the differential theorem can quantify over a
  runtime-host spec rather than a specific implementation.

So the **W5+ path** is: implement the cdz-runtime `"heap"` interface as clean-room Lean `HostFn`s modeling its
**observable** semantics from the spec (`deterministic-value-form.md` + the heap/collections semantics). The
real cost is the memory ABI (how handles and values sit in linear memory). **Do NOT** satisfy the imports by
linking the *real* runtime wasm: that would make the oracle share the runtime with rcdzc, destroying the
independence the differential depends on (a runtime bug would be invisible to both sides). Native host
functions keep the oracle independent.

## Perceus soundness — a verification DIMENSION the heap host must capture (operator, 2026-08-31)

The emitted program manages memory by *calling* the runtime's `dup`/`drop`/`alloc`/access as imports — so the
heap host IS the natural place to verify the emitted dup/drop discipline is memory-sound: **no
use-after-free, no double-free, no leaks.** DESIGN CONSTRAINT on the W5 heap host: make its state `α`
**refcount-and-liveness-aware from the start**, so soundness is a byproduct of every heap-case run, not a
bolt-on:

- `α` : `handle → (value, refcount, live | freed)` (+ Perceus reuse tokens). `alloc`→rc 1; `dup`→rc++;
  `drop`→rc-- and at 0 mark `freed` + recursively drop children.
- **UAF**: any access/dup/drop of a `freed`/unknown handle ⇒ `HostResult.Trap` ⇒ the case surfaces `.trap`.
- **Double-free**: `drop` of an already-`freed`/rc-0 handle ⇒ `.Trap`.
- **Leak**: `runSteps` returns the final store; at program end assert the live-set is empty — any still-`live`
  handle is a leak.
- **⚠ hardest bit = REUSE SPECIALIZATION** (Perceus in-place reuse when rc==1 + reuse tokens threaded through
  alloc): must be modeled faithfully from the spec — a reuse bug is precisely a subtle UAF/aliasing class, so
  it is the highest-value thing to get right.

Two strengths: (a) **dynamic** — per-input, on corpus cases: nearly free once the heap host tracks
rc+liveness; an INDEPENDENT clean-room mirror of the fleet's debug-counters runtime (`assert_node_live` +
`live-objects` census), so an oracle-vs-debug-runtime divergence is a real finding
(dup/drop-insertion bug | runtime bug | spec ambiguity). (b) **symbolic** — no-UAF/no-double-free/no-leak
FOR ALL INPUTS via talos's WP calculus + a `HostContract` encoding the refcount invariant: the aspirational
capstone (research-grade), theorem-shaped ⇒ co-owned with v-lean-oracle's WP layer, a second theorem
dimension alongside value-equivalence. Sequences after W5 (heap host); capturing it now makes rc+liveness a
W5 design constraint.

## W5 scoping — banked design (import surface + increment plan)

Banked 2026-09-02 after the core mission closed (full-corpus 0-diverge: 4275 agree / 0 diverge / 4197 skip of
8472; **2707 of the 4197 skips are heap/runtime-importing cases** — the single biggest gap, what W5 covers).
Concierge greenlit W5 as the next increment; this section is the design bank for a fresh-context reboot of
the heavy multi-increment build.

### The import surface — 100 ops on `"heap"` (from `rcdzc/src/backend/wasm/runtime_abi.rs`)

The emitted wasm imports (by NAME, resolved per-program — no baked op index) a subset of these 100 value-heap
runtime ops. Talos declines any module with imports today; W5 supplies them as clean-room Lean `HostFn`s.
Categories (all params/results are `u32` handles / scalars per the ABI — a handle is a `u32` into the host):

- **Refcount core:** `dup(h)`, `drop(h)` — the Perceus ops. `mark-immortal(h)`, `mark-immortal-deep(h)`,
  `live-objects() -> u32` (census — the leak oracle), `reset()`.
- **Boxing:** `box-int/box-float/box-float32/box-bool`, `get-int/get-float/get-float32/get-bool`.
- **Arrays:** `arr-alloc(n)`, `arr-alloc-reuse`, `arr-get(h,i)`, `arr-set(h,i,v)`, `arr-len(h)`.
- **Vecs:** `vec-empty/of-arr/push/prepend/concat/split/get/len/update/drop`.
- **Maps:** `map-alloc/empty/insert/lookup/remove/set/get/merge/len/size/key/val/to-list` + iter
  (`map-iter/-next/-key/-val`).
- **Sets:** `set-empty/insert/contains/remove/union/intersection/difference/size/to-list` + iter
  (`set-iter/-next/-elem`).
- **Sums (variants):** `sum-new`, `sum-new-reuse`, `sum-disc(h)`, `sum-payload(h)`.
- **Bytes/strings:** `bytes-alloc/get/set/len/slice/concat/compact/scalar-at`, `str-new/from-bytes/get/
  nfc-normalize`.
- **Bignum:** `bigint-of-i64/of-bytes/to-i64-checked/add/sub/mul/div/rem/cmp`,
  `rational-of/num/den/add/sub/mul/div/cmp`.
- **Value transport / AST / hash:** `value-encode/decode/canonicalize/cmp/eq/eq-shaped`,
  `ast-encode/decode/print`, `hash-blake3`.

### Host-state design (rc + liveness aware from the start — the operator's Perceus constraint)

`Store α` with `α = HeapState`: `handle(u32) → { value : HeapValue, rc : Nat, status : Live | Freed,
immortal : Bool }` (+ Perceus reuse tokens). Per-op:
- **alloc** (`arr-alloc`, `map-*`, `set-*`, `sum-new`, `box-*`, `bytes-alloc`, …): fresh handle, `rc := 1`,
  `status := Live`; `value` per the spec's semantics for that op.
- **dup(h)**: `rc++` (require `Live` — else UAF trap). **drop(h)**: `rc--`; at 0 → `status := Freed` +
  recursively drop child handles.
- **access** (`arr-get`, `map-lookup`, `get-int`, `sum-payload`, …): require `Live` — a `Freed`/unknown
  handle ⇒ `HostResult.Trap` (**UAF**). **drop** of `Freed`/rc-0 ⇒ Trap (**double-free**).
- **mark-immortal**: `immortal := true` (excluded from the leak check). **live-objects**: count `Live &&
  !immortal`. **Leak**: at program end, that count must be 0 — else a leak finding.
- **⚠ REUSE SPECIALIZATION** (`arr-alloc-reuse`, `sum-new-reuse`, `vec-update`): rc==1 in-place reuse +
  reuse tokens threaded through alloc. The hardest + highest-value bit — a reuse bug is a subtle
  UAF/aliasing class. Model faithfully from the spec.

Independence: implement `HeapValue` + op semantics from the SPEC (`deterministic-value-form.md` + heap/
collection semantics), NOT by linking the real cdz-runtime wasm — else the oracle shares a runtime with rcdzc
and a runtime bug hides on both sides (the whole point of the differential).

### Increment breakdown (each: implement the HostFn's → those heap cases move skip→runnable; land + weekly picks up)

- **W5.1 — rc/liveness host core + arrays + boxing:** `HeapState`, `dup/drop/mark-immortal/live-objects/
  reset`, `box-*/get-*`, `arr-*`. Wire the `HostRegistry` (name→HostFn over `m.imports`) into `talosDriver`
  (drop the "reject imports" early-return for modeled imports). First end-to-end HEAP differential + the
  Perceus witness (UAF/double-free trap + end-of-run empty-live-set assertion). Proves the whole approach.
- **W5.2 — maps + sets** (incl. iterators). **W5.3 — bytes/strings + bigint/rational.** **W5.4 — sums +
  vecs + the REUSE specialization** (the hard part; highest soundness value). **W5.5 — value-*/ast-*/hash**
  (transport + canonical compare).
- **W6 (dynamic Perceus):** every heap-case run already witnesses no-UAF/double-free/leak (free once W5's
  host tracks rc+liveness). **W7 (symbolic):** WP + a `HostContract` rc-invariant — the aspirational capstone,
  co-owned w/ v-lean-oracle.

The `talosDriver` seam change (W5.1) is the pivot: it must build a `HostEnv`/`HostRegistry` from the modeled
ops keyed by `m.imports` name, and only decline imports it does NOT model (so partial coverage still runs the
scalar parts). Everything downstream (result decode, differential) is unchanged.

### W5.1a landed (host core) + W5.1b op semantics CONFIRMED from the WIT spec

**W5.1a (`Oracle/Wasm/HeapHost.lean`, PR #7960):** `HeapState` (handle-indexed pool of
`{value, rc, live, immortal}`) + the unambiguous ops — refcount core `dup`/`drop`/`live-objects` + boxing
`box-*`/`get-*` — modeled clean-room from the observable value form, with UAF/double-free traps + the leak
census. `w51aHeapOps` keys each by its exact emitted name/sig. `talosDriver` unchanged (W5.1c wires the
registry). Witnessed at the pure `HeapState` layer.

**Deferred-op semantics, now spec-confirmed from `cdz-runtime/wit/runtime.wit` (so W5.1b needs no guessing):**
- **`arr-*` (idx 6–9, the fixed-arity tuple/record product):** `arr-alloc(len) → u32` = array of `len` NULL
  handle-slots (rc 1). `arr-set(arr, i, elem) → u32` sets slot `i`, **stores `elem` WITHOUT dup** (ownership
  MOVE — the array now owns it), returns the *array* handle (threading). `arr-get(arr, i) → u32` returns the
  slot handle; **out-of-bounds TRAPS** (fail-fast). `arr-len(arr) → u32`. ⇒ W5.1b needs a `HeapValue.array
  (Array UInt32)` (0 = null slot) and **`drop` must recursively drop non-null child slots** (the child-drop
  cascade deferred in W5.1a). OPEN (resolve at W5.1b write-time from the `arr-get` op impl / compiler emit):
  does `arr-get` dup the returned handle, or borrow (compiler emits the caller-side dup)? Determines rc balance.
- **`mark-immortal(h) → h` (95):** rc → sentinel `u32::MAX` ⇒ `dup`/`drop` become **NO-OPS** on it + it is
  **excluded from the census** ("counted at alloc; marking decrements the census so an immortal nets to zero" —
  our `liveCount` already excludes `immortal`, so this is: set `immortal := true`, return the handle). ⇒ W5.1b
  must make `dup`/`drop` no-op when `immortal`. `mark-immortal-deep(h) → h` (96) = the transitive version over
  child handles (idempotent + DAG-safe: skip already-immortal); needs the container child-set, so it rides the
  `arr-*`/list/map ops.
- **`reset(node) → u32` + `arr-alloc-reuse(len, token)` / `sum-new-reuse(disc, payload, token)` (26–28) — the
  Perceus REUSE specialization:** `reset` on a UNIQUE (`rc==1`) node drops its children, retains the emptied
  shell, returns it as a non-null reuse token (same handle, childless, rc 1); on a SHARED node decrements +
  returns 0 (null token). `*-reuse(…, token)` refits `token`'s shell if non-null, else allocs fresh; **token 0
  makes them behave exactly as the non-reuse forms**. This is the hard/high-value bit → **W5.4**; until then a
  module importing `reset`/`*-reuse` is simply not covered → skips (sound). A first-cut correctness shortcut:
  model `arr-alloc-reuse(len, 0)`/`sum-new-reuse(…, 0)` = the plain alloc, and `reset` as a plain `drop` that
  returns null token — value-correct whenever the compiler passes token 0, only losing the in-place-reuse
  *aliasing* check (the real W5.4 target).
- **🔑 `live-objects` (54) is NEVER emitted by the compiler** (absent from its allow-list; it's a
  VERIFICATION AID a harness reads *after* `run()`), and in the shipped build it returns **0 unless the
  `debug-counters` feature**. So the leak oracle is NOT a `HostFn` call in any corpus module — it is a
  **post-run inspection of our own `HeapState.liveCount == 0`** (W6). The `w51aHeapOps` `live-objects` entry is
  harmless-but-dead (no emitted module imports it); the real leak assertion lives in the driver/differential.

### W5.2 landed (maps + sets + list core) + the IMMEDIATES ABI finding

**W5.2 surface (`Oracle/Wasm/HeapHost.lean`, on main):** the collection ops, modeled clean-room over the same
value-eq + dup-and-drop machinery as the arrays:
- **Maps** — positional literal-construction (`map` node = interleaved `[k0,v0,k1,v1,…]`) + the functional
  READ-ONLY ops (`map-empty`/`-lookup`/`-size`, key match = structural `valueEq`, borrow) + the CONSUMING ops
  (`map-insert`/`-remove`/`-merge`) with ownership transfer by dup-and-drop per v-runtime's champ.rs contract
  (last-write-wins; `map-merge` b-wins, folds b's dup'd pairs into a then drops b).
- **Sets** — mirror maps at stride 1 (one element handle per entry, no value column): `set-empty`/`-contains`/
  `-size` (borrow) + `set-insert`/`-remove` (consume) + the 2-set ops `set-union`/`-intersection`/`-difference`
  (each CONSUMES both; union = insert-all-of-b with dedup-drop, intersection/difference filter a then free the
  rest of a + all of b).
- **List core (`vec-*`, #8127)** — the growable sequence (`vec-empty`/`-len`/`-get`/`-push`/`-update`),
  immediate-aware; unblocks `map`/`set`-`to-list` (returns a `List` → needs the vec model).

**🔑 The IMMEDIATES ABI finding (#8116 — the W5.2 root-cause fix).** A full-corpus run surfaced 76 DIVERGE +
27 LEAK the moment maps/sets went in. Root cause (read from `cdz-runtime` lib.rs:735–835): the runtime does NOT
heap-allocate small scalars — handles are **tagged**, low 2 bits: `00` heap/NULL, `01` fixnum int (value =
`h>>2`, window ±2^29), `10` atom (unit / bool). `box-int(fixnum)`/`box-bool`/`arr-alloc(0)`→imm-unit produce
**inline immediates**: dup/drop NO-OPS, **census-excluded**. My model heap-allocated them → false leaks (wasm
never drops an immediate) + wrong map-key value-eq (immediate handles read as heap indices). Fixed by modeling
the exact tag ABI (`isImmediate`/`immInt`/`immBool`/`immUnit` + signed fixnum decode) and reworking the pool to
a `(index+1)<<2` heap-handle scheme that never collides with a tag. **LESSON banked:** validate the model at
full-corpus scale *before* extending it — the immediates bug was invisible at the witness layer.

**Witness coverage (on main + #8134 in-flight):** immediates (`probeImmInt`/`-Neg`/`-NoLeak`/`-DupDrop`,
`probeBool`, `probeImmUnit`), heap round-trip + UAF/double-free (`probeHeapInt`/`-UseAfterFree`/`-Cascade`),
immediate-elem containers (`probeArrImmElems`/`-MapImmKeys`/`-VecImmElems`), list core (`probeVec*`), and the
CONSUME-op leak-balance witnesses (`probeMapMerge` b-wins + `probeSet{Union,Intersection,Difference}`, #8134):
each drops the result and asserts `liveCount == 0` (the Perceus property) alongside result size.

### W5.2c — enumeration (`to-list` implemented; the raw-cursor ruling)

**`to-list` is the ONLY program-observable enumeration** (v-runtime, authoritative from champ.rs / value_codec.rs
/ prelude): the language has NO `Map.fold`/`iter`/`keys`/`values`/`first` / `Set.fold` — the sole enumeration
op is `Set.to-list : (Set a) -> (List a)` / `Map.to-list : (Map k v) -> (List (Tuple k v))`. So the enumeration
order a program can observe is `to-list`'s CANONICAL value-sorted order, NOT the CHAMP hash order.

- **`map-to-list(m, desc)` / `set-to-list(s, desc)` (indices 84/83, IMPLEMENTED — PR #8152):** BORROW the
  collection (and `desc`, the compiler-baked shape descriptor, which the model IGNORES — for a scalar key/elem
  the order IS the value order). Return a fresh owned `List`: `set-to-list` → `List a` in canonical element
  order; `map-to-list` → `List (Tuple k v)` where each entry is a fresh 2-element array `[k, v]` in canonical
  KEY order, each `k`/`v` dup'd (co-owned alongside the still-live collection, matching `op_map_to_list`).
  Canonical order = v-lean-oracle's `cmpValue` (Int signed, Bool false<true; String/Bytes raw-byte lexicographic
  arrive with W5.3). An unorderable (non-scalar) key never reaches the model — the compiler rejects it upstream
  (CDZ0203). Modeled via `scalarOrdKey`/`keyLe`/`sortBy` (immediate-aware); witnessed sorted + leak-balanced.
- **🔑 The raw CHAMP cursor (`map-iter`/`-next`/`-key`/`-val` 42–45, `set-iter`/`-next`/`-elem` 51–53) is
  `lowerable:true` (emittable) but its hash order is NEVER program-observable** — it is only ever consumed by
  order-INSENSITIVE folds (sum/count/membership/structural-eq) and the runtime's own walk-then-sort `to-list`.
  ⇒ the oracle models the cursor in CANONICAL SORTED order (agrees regardless of the real hash order) and does
  NOT skip cursor-importing modules. Cursor semantics: `iter` borrows the collection (dups the root); `-next`
  CONSUMES the cursor and returns the advanced one (FBIP at rc 1 / path-copy at rc>1); `-key`/`-val`/`-elem`
  BORROW; a key/elem is NULL only when exhausted. The cursor owns its descent-path frames (reclaimed by the
  standard cascade). Modeling it is a W5.2c follow-up (after `to-list` lands).

### W5.3 / W5.4 landed — the heap-op CORE is complete

All emitted (`lowerable`) heap ops except the `ast-*` binary-AST codec are now modeled (each clean-room from
the WIT + v-runtime's rc.rs/scalars.rs/champ.rs contracts, witnessed, leak-balanced) — incl. `bigint-of-bytes`
(index 82, sign-magnitude `[sign][LE mag]` → `Int` leaf, consumes buf) and `value-eq`/`value-eq-shaped` (W5.5).
`value-cmp`/`value-canonicalize` are parked (Core doesn't model them as ops yet → Core-side skips). Summary:

- **Bytes + strings (W5.3):** `bytes-alloc/set/get/len/scalar-at`, `str-from-bytes` (a flat `Array UInt8` leaf;
  String == Bytes share one heap rep — the Str/Bytes split is only a value-encode descriptor), and the rope
  `bytes-concat/slice/compact` modeled **flat** — value-faithful AND leak-VERDICT-faithful (the leak oracle
  thresholds `leak > 0`, not an exact count, and flat frees eagerly so `flat_census==0 ⇔ rope_census==0` for
  consume-semantics ops; the slice-pins-parent sharing only shifts the count on an already-leaking run).
- **Sums (tagged variants):** `sum-new/-disc/-payload` — `HeapValue.sum (disc, payload)`, arity-1 (payload
  child); a nullary variant carries the unit immediate.
- **Numeric:** BigInt (`of-i64/to-i64-checked/add/sub/mul/div/rem/cmp`) = a Lean `Int` leaf, ALWAYS heap (zero
  is a fresh heap leaf, never null; null READS as 0); Rational (`of/num/den/add/sub/mul/div/cmp`) = a normalized
  `[num,den]` node. **Borrow-heavy** (opposite of the CHAMP collection ops): every arith/cmp/convert BORROWS +
  fresh owned result; only `rational-of` consumes.
- **List extras:** `vec-concat/-prepend/-of-arr/-drop` (flat, consume).
- **Reuse / FBIP (W5.4, the W7 crux):** `reset` (rc==1→drop-children+keep-shell-return-same-handle;
  rc>1/immortal→NULL), `arr-alloc-reuse`/`sum-new-reuse` (refit the token or alloc fresh). The `rc==1` gate +
  dup-before-drop makes BOTH dimensions catch a broken uniqueness gate (aliased-read/UAF value divergence).

### W5.5 — `value-*` (the last cluster): plan + two divergence-risk gaps

`value-eq` (61) is DONE — `valueEqOp` wraps `valueEq` (BORROWS both, like `set-contains`), now that `valueEq`
is order-independent + total (Gap 2 closed). `value-eq-shaped` (88) also wraps structural `valueEq`; `value-canonicalize`
(87) is identity/dup for canonical values; `value-encode`/`-decode` (the binary-AST byte codec, `cdzast\x00\x01`
header) are DEFERRED (a `value-encode` result is non-scalar Bytes → low coverage). `value-cmp` (86) is the
blessed three-way order = v-lean-oracle's `cmpValue` STRUCTURE (Int signed / Bool false<true / Str-Char-Bytes
`cmpBytes` lex / Rational `a·d` vs `c·b` / Tuple-List-Set lex-over-children / Record + Map sorted-by-key then
lex), same-type-only (cross-type `valRank` unobservable), DECLINES on floats. **TWO GAPS to resolve first:**
1. **Sum ordering is NOT the numeric disc.** Option/Result have fixed ranks (Some<None, Ok<Err); user variants
   order by TAG-NAME bytes. `HeapValue.sum` stores only a numeric disc (declaration order) → `value-cmp` on sums
   needs the `desc` (variant names) or a confirmed disc convention. Defer `value-cmp`-on-sums.
2. **`valueEq` compares set/map POSITIONALLY** (order-sensitive), but sets/maps are UNORDERED — two equal
   sets/maps built in different insertion orders would false-unequal. Fix `valueEq` to be order-independent for
   set/map (mutual containment via `valueEq`; no total order needed) — a real latent bug beyond `value-*`.
   - PARTIAL PROGRESS: the `.vec` (List) arm was MISSING (two lists fell through to `false`); added as a
     POSITIONAL compare (lists ARE ordered, mirrors `.array`/Tuple) — clean, worklist-based, no termination risk.
   - DONE (set/map order-independence): CANONICALIZE-then-positional, NOT a containment search — dodges the
     termination trip AND the decode/import cycle. KEY: set elems + map keys are ALWAYS SCALAR (compiler rejects
     compound keys, CDZ0203), so a total order over SCALAR keys suffices. `.set` arm = `sortBy id` both element
     lists then positional; `.map` arm = pair the flat `[k,v,…]`, `sortBy` key, compare `(k,v)` positionally
     (values recurse). `keyLe` extended with a bytes-lex arm (`cmpBytesLex` == Core's `cmpBytes`: unsigned,
     prefix<longer) so String/Bytes keys order canonically too (Bool/Int order unchanged; also fixes
     set-to-list/map-to-list string-key order consistently). Cross-rank never occurs (homogeneous keys), so only
     same-type order must match cmpValue — confirmed by v-lean-oracle (Str/Bytes=cmpBytes; Char=codepoint/UTF-8
     lex). Witnesses: Int-set + String-set + Int-map different-insertion-order equality + `cmpBytesLex`.

### W5.6 — heap-VALUED RESULT decode (COMPLETE for built-ins)

A heap-valued `main` returns an i32 HANDLE, not a scalar; the final `HeapState` at that handle lets us
RECONSTRUCT the value instead of skipping the case. Two stages:

1. **Structural decode** (`Oracle.Wasm.HeapDecode.decodeValue?`, type-AGNOSTIC): `handle + HeapState → raw
   Oracle.Value`. Immediates → `.int`/`.bool`/`.unit`; heap leaves → `.int`(int/bigint) / `.f64` / `.bytes` /
   `.rational`; `.array`→`.tuple`, `.vec`→`.list`, `.set`→`.set`, `.map`→`.map` (children recursed, RAW order);
   a `.sum (disc,payload)` → an INTERMEDIATE `.variant "<decimal-disc>" payload` (the disc→name map needs the
   type, so the fixup finishes it). `partial mutual` with EXPLICIT list helpers — never `List.mapM` over `Option`
   (that trips the `native_decide` "uses sorry" codegen).

2. **Result-type-directed fixup** (`resultTyFixup` → `fixupTy`, in `Oracle.Wasm`), applied at the Outcome
   boundary (`toOutcomeHeap : (Value → Option Value) → WasmOutcome → Outcome`). The decoder is type-agnostic, so
   `fixupTy` walks the entry's `cdz-result-type` node in parallel with the decoded value and retags where the
   type demands: **String** `.bytes`→`.str` (Str/Bytes share the byte rep); nested String recursively through
   **List/Set** (element), **Tuple** (positional), **Map** (key/value); **Record** `.tuple`→`.record` via the
   type's KEY-SORTED `(: name type)` fields; **Option/Result** intermediate `.variant`→Core's dedicated
   `.some/.none/.ok/.err` by discriminant (Some=0/None=1, Ok=0/Err=1 declaration order; Option's payload-type
   list is SPARSE = Some only, Result's dense = [Ok,Err]). A String/variant in a position the fixup can't
   reconstruct → `none` = SOUND SKIP (never a false-diverge).

`decodableHeapHead` = the recognized single-head heads (BigInt/List/Map/Set/Rational/Bytes/Record/Sum; String
via `stringResult?`; the flat multi-value TUPLE form too) that route a result to the driver + `resultTyFixup`.

**Decoded now:** scalars, BigInt, Bytes, String (+ arbitrarily nested), List, Set, Tuple, Map, Rational, Record,
Option, Result. **Still declines (sound skip):** USER sums (variant names are NOT in the emitted
`cdz-result-type` — they live behind the type's `declId`; decoding them needs a COMPILER EMIT EXTENSION to carry
variant names, route to the emit owner), Ordering/Sign (own Value repr), Nominal/Qty, and the `ast-*` codec.

## Gate coverage

`Oracle.Wasm`'s invariants are pinned by compiled `example` witnesses in the module (no corpus case
exercises this internal boundary; per `PRINCIPLES.md` §2 that is exactly what a Lean check is for). The
oracle-lean build (and thus the nix `.#checks.<sys>.oracle-lean-*` checks) compiles them, so a regression
fails the build. NOTE: the oracle-lean checks are **advisory** — they are NOT in `localGate`'s required set,
so gate an oracle-lean slice via the `.#checks.<sys>.oracle-lean-{smoke,check,ast-roundtrip}` attrs directly,
not `cargo xtask fleet gate-local` (which neither builds nor protects oracle-lean).
