# PR #1616 review comments — cdz-nfc + rcdzc + xtask (v-runtime) — MERGED, fix-forward

Mirrored from https://github.com/camshaft/cadenza/pull/1616 (PR: "[v-runtime] a900cf39c", MERGED
2026-08-03). 6 Copilot comments accrued just before merge — all doc-drift on the FINDING#23 NFC work.
Two coherent themes; both need a fix-FORWARD (PR is already on trunk).

## Theme A — doc OVERSTATES the emit coverage (3 sites) — doc/accuracy [more substantive]
- cdz-nfc/wit/nfc.wit:13 — comment claims NFC emitted for "String champ-key construction" + "symbol-intern" generally
- rcdzc/src/core.rs:501 — `Core::NfcNormalize` doc says emitted for "String Map/Set key" construction
- rcdzc/src/backend/wasm/select.rs:264 — `str-nfc-normalize` comment says "String Map/Set key" sites

> The only current `Core::NfcNormalize` construction sites in this PR are `String.concat` (runtime) and
> `Symbol.of` (runtime). The comments should be narrowed to the implemented sites to avoid overstating
> coverage.

The docs imply NFC normalization happens at Map/Set-key and champ-key construction, but only
`String.concat` + `Symbol.of` lowering actually emit it. This is more than cosmetic — a reader/maintainer
could assume String Map/Set keys are NFC-normalized when they are NOT (a correctness-relevant gap in
mental model). Narrow the 3 comments to the implemented sites (String.concat + Symbol.of), OR if
Map/Set-key normalization is intended-but-unimplemented, file it as a tracked follow-up rather than
doc-claiming it. RECOMMEND verifying which was meant.

## Theme B — versioned-import-name vs plain runtime-dep model (3 sites) — doc [same as #1580/#1590 pts 2/3]
- cdz-nfc/src/lib.rs:7, xtask/codegen.rs:132, cdz-nfc/wit/nfc.wit:6

> The docs say the emitted PROGRAM imports `cadenza:nfc/normalize` by hash / the compiler "pins" a
> versioned `cadenza:nfc/normalize@…+<hash>` name — but the runtime WIT imports the PLAIN
> `cadenza:nfc/normalize`, and the HOST composes the NFC component into the runtime via
> `REQUIRED_NFC_HASH`. No versioned `@…+<hash>` name is used anywhere.

Same doc-drift this liaison filed on #1580 pts 2/3 and #1590 — the "program imports a per-program
versioned @…+<hash> name" doc contradicts the actual runtime-world plain-name dep + host-composition
model. Reconcile all 3 comments to the real flow. Doc-only, LOW.
