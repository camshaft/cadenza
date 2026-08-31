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

## The interpreter: talos (pinned, being integrated)

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
- **W0 — toolchain + talos pin** (co-owned w/ v-nix; `lean4_432` landed #7162; atomic co-land pending the
  Mathlib-in-nix spike): bump `lean-toolchain` → v4.32.2, add the talos flake input + lakefile `require`,
  flip oracleLean to `lean4_432` — all atomically.
- **W2/W3 — talos drive** (blocked on W0): wire `runWasm` to drive talos's lib (`Wasm.Decoder.Wat` +
  `Wasm.SmallStep`) on the extracted core module, producing a `WasmOutcome` for `toOutcome`.
- **W4 — self-contained scalar/arith subset**: end-to-end differential over zero-import core modules
  (align with compiler-ml's `run-emitted` set); widen verified `ScalarTy` spellings from real emits.
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

## Gate coverage

`Oracle.Wasm`'s invariants are pinned by compiled `example` witnesses in the module (no corpus case
exercises this internal boundary; per `PRINCIPLES.md` §2 that is exactly what a Lean check is for). The
oracle-lean build (and thus the nix `.#checks.<sys>.oracle-lean-*` checks) compiles them, so a regression
fails the build. NOTE: the oracle-lean checks are **advisory** — they are NOT in `localGate`'s required set,
so gate an oracle-lean slice via the `.#checks.<sys>.oracle-lean-{smoke,check,ast-roundtrip}` attrs directly,
not `cargo xtask fleet gate-local` (which neither builds nor protects oracle-lean).
