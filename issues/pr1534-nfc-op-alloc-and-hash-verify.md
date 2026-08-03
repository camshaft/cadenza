# PR #1534 review comments — cdz-runtime/src/lib.rs + cdz-run/src/cli.rs + xtask/codegen.rs (v-runtime)

Mirrored from https://github.com/camshaft/cadenza/pull/1534 (PR: "[v-runtime] 5a011c1ec").
This is the str-nfc-normalize op work (finding #23 fix).

## 1. `op_str_nfc` always allocates even when input is already NFC (Copilot, lib.rs:3183) — perf/contract
> `op_str_nfc` always allocates a fresh String leaf and drops the input, even when the input bytes
> are already NFC. Several call sites/docs in this PR state the op returns the same handle (and is
> near-free) for already-NFC text; the current implementation contradicts that and adds avoidable
> heap churn on the common already-NFC path (e.g., ASCII).

The docs promise "same handle, near-free for already-NFC" but the impl always allocates — fast-path
the already-NFC case (return the input handle when the NFC check finds no change) so the common
ASCII/already-NFC path doesn't churn the heap, matching the stated contract.

## 2. `resolve_nfc` doesn't verify the loaded bytes hash to the manifest `nfc` address (Copilot, cli.rs:342) — correctness/integrity
> `resolve_nfc` loads `<store>/<hash>.wasm` but does not verify that the bytes actually hash back to
> the manifest's `nfc` content address. `resolve_runtime` performs this verification to prevent
> silent substitution/corruption; NFC should do the same for consistency and to match the comment
> that hash verification occurs later (it currently doesn't).

Mirror `resolve_runtime`'s content-address verification in `resolve_nfc` — otherwise a corrupted/
substituted `<hash>.wasm` loads silently, and the "hash verified later" comment is false.

## 3. `resolve_ops` doc says "IN DECLARATION ORDER" but sorts by name (Copilot, codegen.rs:322) — doc
> The `resolve_ops` doc comment says ops are returned "IN DECLARATION ORDER", but the function
> explicitly sorts by name (`ops.sort_by(...)`). Update the comment to the actual sorted-by-name
> contract.

Points 1+2 are the substantive ones (perf-contract mismatch + missing integrity check on a
content-addressed load); 3 is doc-drift.
