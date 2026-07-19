# SMITH FINDING — backend emitted INVALID wasm: type mismatch: expected i64, found i32 (at offset 0x123)

_Filed by `cdz-smith` (the compiler fuzzer). This is an auto-generated finding: a
generated program made the compiler PANIC, HANG, or emit INVALID wasm — never valid
behavior, since the compiler reports every legitimate "no" as a diagnostic. Triage, fix,
then rename this file `.RESOLVED.md` (or `.REJECTED.md` with a rationale) so it is not
re-triaged._

- **Category:** invalid-wasm
- **Compiler commit:** `c9940747e`
- **Hits:** 1
- **Signature:** `invalid-wasm-type-mismatch-expected-i-found-i-at-offset`

## Reproducer

`invalid-wasm-type-mismatch-expected-i-found-i-at-offset.smith.sexp` (also inline):

```scheme
(do (def (main) (match ((fn (v0) (* (tuple) 0)) 0) (_ 0))) (export main))
```

Reproduce in-process:

```
cargo run -p cdz-smith -- verify invalid-wasm-type-mismatch-expected-i-found-i-at-offset.smith.sexp
```

## Invalid wasm (backend miscompile)

The compiler reported SUCCESS, but the emitted component failed wasm validation
(`wasmparser` with `WasmFeatures::all()` — the same check rcdzc's own tests assert
emitted components pass). The backend produced structurally-invalid wasm.

- **Validator error:** type mismatch: expected i64, found i32 (at offset 0x123)

## RESOLVED 2026-07-19 (v-inference)
FIXED on trunk (verified @ 80bfe936e, fuzzer filed @ c9940747e). `(* (tuple) 0)` — and the fuzzer's `(match ((fn (v0) (* (tuple) 0)) 0) (_ 0))` — now REJECTS AT CHECK with CDZ0201 ("a (Tuple) and an Int64 are different types … across that kind boundary"), no invalid wasm emitted. The check-vs-compile gap is closed: the arithmetic-operand numeric-requirement check catches the compound operand before emit. PINNED as a regression witness: spec/semantics/07-type-system.sexp "multiplying a compound (tuple) by a number is a cross-kind type error, not invalid wasm" (CDZ0201 reject, +1 all three baselines). v-inference (diagnostics/inference lane).
