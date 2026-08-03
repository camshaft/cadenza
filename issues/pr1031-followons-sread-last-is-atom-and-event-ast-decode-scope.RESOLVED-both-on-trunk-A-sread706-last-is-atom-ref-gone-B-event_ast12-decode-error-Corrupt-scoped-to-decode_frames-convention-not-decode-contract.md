# PR#1031 — two doc follow-ons on the PR#1026 (sread.cdz) and PR#1027 (event_ast.rs) fixes

Two Copilot review comments, split by owner. Both are DOC-only residuals introduced by the just-landed
PR#1026/PR#1027 fixes. Gate: sread.cdz = compiler-ml self-host + `cargo test -p rcdzc`; event_ast.rs =
cdz-kernel own `cargo test`+clippy.

## Comment A (verbatim) — sread.cdz:698 (id 3696601459) → v-compiler-ml
blame `df2cb2915` (the PR#1026 record-field-type-unbound reject-gap fix, 7d8774059's landed form).

- "The `record-field-type-unbound` doc comment mentions `last-is-atom`, but the function doesn't have
  such a parameter/variable. This makes the comment misleading; `last` already carries the current
  candidate type atom/head."

### Liaison verification (confirmed on trunk e68033e83)

sread.cdz:698 doc: "`last-is-atom` tracks whether the pending `last` is a bare atom (check `last` on
close) vs already-resolved." But `record-field-type-unbound(s: String, g: Int64, last: String, tree:
Tree)` (:699) has NO `last-is-atom` parameter or local — the PR#1026 fix reworded the doc and left a
dangling reference to a variable that doesn't exist (the design was presumably simplified to just carry
`last`). `last` alone carries the candidate type atom/head; there's no separate is-atom flag. Fix: delete
the `last-is-atom` sentence (or reword to describe `last` as it actually is). Doc-only.

## Comment B (verbatim) — event_ast.rs:13 (id 3696601467) → v-agent-harness
blame `df2cb2915`-adjacent (the PR#1027 torn-vs-corrupt reword, 68ebc26e5's landed form).

- "The updated module docs say the framing layer decides torn vs corrupt and that any `decode` failure on
  a complete frame is corruption, but the wording 'this codec's error only means … Corrupt' is stronger
  than the actual API contract of `decode`/`EventAstError` (which can still surface codec `Truncated`
  when called on unframed/truncated bytes). Consider rephrasing this block to explicitly scope the
  'decode error ⇒ Corrupt' rule to `log_store::decode_frames` on complete frames, to avoid contradicting
  later docs in this module."

### Liaison verification (confirmed on trunk e68033e83)

event_ast.rs:12-13 (the PR#1027 reword): "So this codec's error only means 'this whole frame is bad'
(→ Corrupt); the torn side is decided before decode is reached." Copilot's nuance is correct: `decode`
called DIRECTLY on truncated/unframed bytes can return codec `Truncated` (via `EventAstError::Codec`) —
which means "not enough bytes", NOT "this whole frame is corrupt". The "decode error ⇒ Corrupt" mapping is
a convention of `log_store::decode_frames` (which only calls `decode` on a COMPLETE frame, so there any
error IS corruption). The module-doc wording states it as an unconditional property of the codec's error,
which over-reaches the `decode`/`EventAstError` contract and can read as contradicting the encoding-shape
docs below that describe `Truncated` as a truncation signal. Fix: scope the "decode error ⇒ Corrupt" rule
explicitly to `decode_frames` on complete frames (e.g. "…so WHEN `decode_frames` calls `decode` on a
complete frame, any error there means the frame is corrupt"), leaving `decode`'s own contract (can surface
`Truncated`) intact. Doc-only precision — the FRAMING attribution the PR#1027 fix added is correct; this
just narrows the over-strong "codec's error only means Corrupt" sentence to the framed-caller context.

Owners: **v-compiler-ml** (sread.cdz:698 — delete/reword the `last-is-atom` reference) + **v-agent-harness**
(event_ast.rs:12-13 — scope the decode-error⇒Corrupt rule to `decode_frames` on complete frames). Both
doc-only residuals on the PR#1026/PR#1027 fixes; code correct in both.
