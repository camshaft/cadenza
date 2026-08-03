# PR #1590 review comments — cdz/src/main.rs + guide/scripts/check-examples.mjs + guide/src/runner/runWorker.ts (v-runtime)

Mirrored from https://github.com/camshaft/cadenza/pull/1590 (PR: "[v-runtime] rcdzc+runtime: NFC-normalize
runtime Strings at construction (FINDING #23) — via a separate cdz-nfc component").
This is the re-dispatched FINDING#23 NFC work (was #1580). Copilot re-flagged the main.rs hash-verify
gap I filed on #1580, PLUS 2 new TextDecoder findings on the JS shims.

## 1. NFC component loaded by hash WITHOUT content-address verification (Copilot, cdz/src/main.rs:6016) — correctness/integrity [ALSO filed on #1580]
> The cdz harness loads the NFC component bytes from `<store>/<hash>.wasm` but does not verify that the
> file's contents actually hash to the manifest's `nfc` content address. This weakens the integrity
> guarantees of the content-addressed store.

SAME gap as my #1534 point 2 (cdz-run cli.rs, fixed there) and #1580 point 1 — the compiler-side
compose path loads `<hash>.wasm` but never checks `sha256(bytes) == hash`. A corrupted/substituted
store entry silently composes the wrong NFC tables. SUBSTANTIVE — reuse the same verify helper before
composing. (Still open on the re-dispatched PR.)

## 2. runWorker.ts NFC shim uses non-fatal TextDecoder — silently replaces invalid UTF-8 (Copilot, guide/src/runner/runWorker.ts:29) — correctness
> The NFC shim uses TextDecoder with the default non-fatal UTF-8 mode, which replaces invalid sequences
> with U+FFFD and would silently change the bytes being normalized.

VERIFIED against the diff: `new TextDecoder("utf-8").decode(bytes)` (default non-fatal) → invalid UTF-8
becomes U+FFFD, then re-encodes DIFFERENT bytes. The runtime contract is well-formed UTF-8, so decode
with `{ fatal: true }` to fail loudly on malformed input instead of silently corrupting. NOTE: this is
a `guide/` file — if v-guide/v-guide-editor owns the shim, coordinate; the PR author (v-runtime) added
it as part of the NFC harness wiring.

## 3. check-examples.mjs NFC shim — same non-fatal TextDecoder issue (Copilot, guide/scripts/check-examples.mjs:148) — correctness
> The NFC shim decodes UTF-8 with the default non-fatal TextDecoder behavior, which can silently
> replace invalid sequences and then re-encode different bytes.

Same as point 2, second shim. Use `new TextDecoder("utf-8", { fatal: true })`. Same guide-ownership
note. LOW (test/harness shim, not production runtime — the real cdz-nfc component does the production
normalization; these JS shims only stand in for the jco harness).
