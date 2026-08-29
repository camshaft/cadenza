# `data/` — language-independent authored source tables

This directory holds **hand-authored, committed source-of-truth data** that the compiler build
DERIVES code from — kept OUTSIDE the Rust compiler tree (`implementation/seed/crates/rcdzc/…`) so it is
**language-independent** (operator ruling, 2026-08-29): the table is the single source; the generated Rust
is just one consumer of it. The flow is always **source → generated code**, never the reverse.

## `wasm-abi.sexp` — the wasm / component-model byte table

The authoritative list of every wasm / component-model byte the backend emits: the core opcode table,
core + component valtype bytes, section ids, the two magic headers, and the functype form bytes. It is a
plain Cadenza s-expression — a `(do …)` of tagged entries:

```
(do
  (opcode I32_CONST 65)                    ; NAME  byte
  (single CORE_I32 127 "core valtype i32") ; NAME  byte  doc
  (magic CORE_MAGIC 0 97 115 109 1 0 0 0 "the \0asm core-module preamble.") ; NAME  b0..b7  doc
  …)
```

Bytes are DECIMAL here (hex formatting is the code generator's job). Every `single`/`magic` carries its
`///` doc so the generated constants stay documented.

### It is HAND-AUTHORED — edit this file directly

`wasm-abi.sexp` is the source of truth; **nothing generates it.** To add or change a byte, edit this file.
Two derived artifacts flow from it (both via `xtask-codegen-wasm-abi`, which `cdz convert`s the sexpr to a
cadenza-ast binary and renders Rust):

- `implementation/seed/crates/rcdzc/src/backend/wasm/wasm_abi.rs` — the module the backend's serializer reads.
- `implementation/seed/crates/wasm-abi-table/` — a standalone crate of the same constants, whose
  `#[cfg(test)] mod oracle` asserts every byte against the `wasm-encoder` spec encoder.

### Guardrails (a wrong byte cannot ship silently)

`wasm-encoder` is the cross-check **oracle**, inverted from the old extract-from-encoder approach: instead
of trusting the encoder to fill bytes, the authored sexpr states them and the tooling asserts they MATCH
the encoder, so a transcription typo in a hand-edit is caught at build:

- the `wasm-abi-oracle` nix check runs `xtask-codegen-wasm-abi --oracle-check` (sexpr bytes vs the encoder);
- the `wasm-abi-table` crate's generated oracle tests assert each constant vs the encoder.

So the workflow is: **edit `wasm-abi.sexp` → regenerate the derived Rust → the oracle catches any mismatch.**
