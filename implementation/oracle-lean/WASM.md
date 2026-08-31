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
- **W5+ — runtime-importing cases**: multi-module components need entry-module identification + cdz-runtime
  (`"heap"`) import modeling/linking. The deep end; scalars first.

## Gate coverage

`Oracle.Wasm`'s invariants are pinned by compiled `example` witnesses in the module (no corpus case
exercises this internal boundary; per `PRINCIPLES.md` §2 that is exactly what a Lean check is for). The
oracle-lean build (and thus the nix `.#checks.<sys>.oracle-lean-*` checks) compiles them, so a regression
fails the build. NOTE: the oracle-lean checks are **advisory** — they are NOT in `localGate`'s required set,
so gate an oracle-lean slice via the `.#checks.<sys>.oracle-lean-{smoke,check,ast-roundtrip}` attrs directly,
not `cargo xtask fleet gate-local` (which neither builds nor protects oracle-lean).
