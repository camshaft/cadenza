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
