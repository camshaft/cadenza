# PR#1013 review comments (×5) — §21b runtime-dependency resolution: multi-import, prefix-match, error-conflation, hex-case, fixture-target (v-agent-harness)

Mirrored from GitHub PR#1013 review comments (Copilot), ids `3695976942` (wasm_host.rs:161),
`3695979853` (:145), `3695979859` (:113), `3695979864` (:185), `3695979871`
(tests/fixtures/reducer-guest/Cargo.toml:4). All `cdz-kernel` → v-agent-harness. Blame `4614bdb75`
"feat(cdz-kernel): runtime-dependency resolution — detect + CAS-fetch a reducer's runtime import (§21b)".
Gate = cdz-kernel's own `cargo test`+clippy (incl `--features live-exec`), NOT `xtask check`.

## Comment 1 (verbatim) — :161, multiple runtime imports silently ignored

- (id 3695976942) "Multiple runtime imports silently ignored. If a component declares multiple
  `cadenza:runtime/heap@...` imports (malformed or corrupted component), only the first match is returned
  (line 155-158). This could cause incomplete linking… Validate that at most one runtime import exists
  and error if multiple are found." [+ suggested loop that errors on a 2nd match]

### Liaison verification (confirmed on trunk ba54cf63d)

`required_runtime` (:145-161) loops the component imports and `return Ok(Some(RuntimeReq…))` on the FIRST
`starts_with(RUNTIME_IFACE)` match. A (malformed/corrupt) component declaring >1 runtime import gets only
the first silently — the rest ignored, so the kernel may link a runtime the component didn't primarily
expect. Copilot's fix (accumulate into an `Option`, error on a 2nd match) is sound defensive validation
(a well-formed reducer has exactly one runtime import). Not a hallucination — the loop + first-match
return are real.

## Comment 2 (verbatim) — :145, prefix false-positive

- (id 3695979853) "`required_runtime` matches imports with `name.starts_with(RUNTIME_IFACE)`, which will
  also match unrelated interfaces like `cadenza:runtime/heap2@...`. Since the intended import name is
  `cadenza:runtime/heap@<semver>+<hash>`, the match should require an `@` immediately after the interface
  id to avoid false positives."

### Liaison verification (confirmed)

`RUNTIME_IFACE` is matched by `name.starts_with(...)`, so `cadenza:runtime/heap2@…` (a hypothetical
sibling) would false-match. Require the `@` version separator right after the id (`starts_with(concat!
(RUNTIME_IFACE, "@"))` or check `name[RUNTIME_IFACE.len()..]` starts `@`). Robustness.

## Comment 3 (verbatim) — :113, error-variant conflation

- (id 3695979859) "`ComponentError::RuntimeUnresolved` is documented as 'missing by hash', but it's also
  used when the blob store returns an I/O error. Either split the error variants (missing vs backend
  error) or update the docs so callers don't mis-handle operational failures as 'not found'."

### Liaison verification (confirmed)

`resolve_runtime_bytes` (:167-183): `Ok(None) => Err(RuntimeUnresolved("not in the blob store"))` AND
`Err(e) => Err(RuntimeUnresolved("blob store error…"))` — a genuine I/O/backend failure is reported as
the same `RuntimeUnresolved` a caller reads as "missing". Split into `RuntimeMissing` vs
`RuntimeStoreError` (or fix the doc). Correctness of error-handling semantics.

## Comment 4 (verbatim) — :185, hex-case doc-vs-impl

- (id 3695979864) "`parse_hash_hex` is documented as parsing '64 lowercase-hex chars', but the
  implementation accepts uppercase hex too (`u8::from_str_radix(..., 16)`). Either enforce lowercase
  explicitly or relax the comment to just 'hex'."

### Liaison verification (confirmed)

Doc "64 lowercase-hex chars" but `from_str_radix(…, 16)` accepts A-F too. Content addresses are canonically
lowercase, so ENFORCE lowercase (reject uppercase — a canonical-form check) or relax the doc; enforcing is
better for content-address canonicity. Doc-or-validation.

## Comment 5 (verbatim) — fixture Cargo.toml:4, target mismatch

- (id 3695979871) "The header comment says the guest is compiled for `wasm32-wasip1`, but the documented
  regeneration recipe + CI both build it for `wasm32-unknown-unknown`. This mismatch makes it hard to
  know which target is actually intended for the fixture."

### Liaison verification (confirmed)

The reducer-guest fixture header says `wasm32-wasip1` but the regen recipe/CI use
`wasm32-unknown-unknown`. Reconcile to the actually-used target. Doc.

Owner: **v-agent-harness** (`cdz-kernel` §21b, `4614bdb75`). 1-2 = defensive validation (multi-import
error + `@`-anchored prefix), 3 = error-variant split, 4-5 = doc/canonicity.
