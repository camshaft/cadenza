# PR #1580 review comments — cdz/src/main.rs + xtask/codegen.rs (v-runtime)

Mirrored from https://github.com/camshaft/cadenza/pull/1580 (PR: "[v-runtime] 44fa249dd").
This is the FINDING#23 NFC-as-separate-component work (operator ruling d: runtime imports a separate
`cdz-nfc` wasm component by hash). Related to my earlier #1534 finding (the `cdz-run` cli.rs resolver
hash-verify gap — which WAS fixed there). Copilot found the SAME gap on a second load path here.

## 1. NFC component loaded by hash WITHOUT content-address verification (Copilot, cdz/src/main.rs:6015) — correctness/integrity
> `nfc` is loaded from `<store>/<hash>.wasm` without verifying that the bytes actually hash to the
> manifest's `nfc` value. If the store entry is corrupted or substituted, this path will silently
> compose the wrong NFC tables (unlike `cdz-run`'s CLI resolver, which verifies the content address).
> Consider sharing a hash-verification helper so this path also checks `sha256(bytes) == hash` before
> composing.

VERIFIED against the diff: the compose-path NFC load reads `store.join("runtime.toml")`, parses the
`nfc = "<hash>"` line, then `.and_then(|hash| std::fs::read(store.join(format!("{hash}.wasm"))).ok())`
— it loads by hash but never checks `sha256(bytes) == hash`. This is the SAME integrity gap this
liaison filed on #1534 point 2 (the `cdz-run` cli.rs `resolve_nfc` path), which was fixed to mirror
`resolve_runtime`'s verification. A corrupted/substituted `<hash>.wasm` in the store silently composes
the wrong NFC tables. SUBSTANTIVE: share/reuse the same content-address-verification helper here before
composing. (Content-addressed loads must verify the address or the CAS guarantee is void.)

## 2. Docstring says program imports versioned `cadenza:nfc/normalize@…+<hash>`, but it's a runtime dep with a plain iface name (Copilot, codegen.rs:505) — doc
> These docstrings say "a program imports" and refer to a versioned `cadenza:nfc/normalize@…+<hash>`
> name, but the WIT change makes this a *runtime* dependency (`cdz-runtime` world imports
> `cadenza:nfc/normalize`).

The WIT wires NFC as a `cdz-runtime`-world import under the PLAIN interface name `cadenza:nfc/
normalize` (NFC_IFACE = "cadenza:nfc/normalize" in the diff), not a per-program versioned `@…+<hash>`
import. Reword the docstring to "the runtime imports" + plain iface. Doc-only, LOW.

## 3. Doc says compiler "pins" REQUIRED_NFC_HASH into a versioned import name, but it composes via the plain name (Copilot, codegen.rs:512) — doc
> This doc block states the compiler "pins" `REQUIRED_NFC_HASH` into a versioned
> `cadenza:nfc/normalize@…+<hash>` import name, but the current implementation composes NFC into the
> runtime using the plain interface name.

Same drift as point 2 — reconcile the "pins into a versioned name" doc with the actual plain-name
runtime composition. Doc-only, LOW. (Points 2+3 are the same root: doc describes a per-program
versioned import; impl is a runtime-level plain-name dep. Point 1 is the substantive one.)
