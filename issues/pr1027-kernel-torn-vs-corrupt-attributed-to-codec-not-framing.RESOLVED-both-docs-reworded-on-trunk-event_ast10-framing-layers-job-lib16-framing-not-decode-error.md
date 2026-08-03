# PR#1027 — cdz-kernel: docs attribute the torn-vs-corrupt split to the codec's DecodeError, but it's the FRAMING layer (v-agent-harness)

Two Copilot review comments, both `cdz-kernel` → v-agent-harness. FOLLOW-ON on the PR#1023 doc fix
(83b7c6dce, landed on trunk 0565a93e4): the reworded docs now over-credit `event_ast::decode`/`DecodeError`
for the torn-vs-corrupt split, when `log_store::decode_frames` actually makes that split at the u32-framing
layer. Doc-only. Gate = cdz-kernel own `cargo test`+clippy.

## Comment 1 (verbatim) — event_ast.rs (id 3696426880)

- "The updated module docs imply the torn-vs-corrupt distinction comes from `decode_detailed`/`DecodeError`,
  but `log_store` actually classifies torn tails at the framing layer (short prefix/body) and treats any
  decode failure on a complete frame as corruption. Rewording this keeps the docs consistent with
  `log_store::decode_frames`' behavior."

## Comment 2 (verbatim) — lib.rs (id 3696426889)

- "This bullet still suggests `event_ast::decode`'s error handling is what preserves the torn-vs-corrupt
  recovery split. In practice, `log_store` uses framing to detect torn tails and then treats decode
  failures on complete frames as corruption. Tweaking the wording here would better match `log_store`'s
  actual recovery logic."

## Liaison verification (confirmed on trunk 0565a93e4)

`decode_frames` (log_store.rs) makes the split by FRAMING, not by `DecodeError`'s variant:
- a short length prefix (<4 trailing bytes) or a body shorter than the prefix claims → `TornTail`
  (torn write, benign) — decided BEFORE calling `event_ast::decode`;
- a COMPLETE frame whose body fails `event_ast::decode` (ANY `Err(_)`, including the codec's own
  `Truncated`) → `Corrupt`.
So it does NOT inspect `DecodeError`'s variant to choose torn-vs-corrupt — the framing layer decides torn,
and any decode error on a whole frame is uniformly corruption. But the post-fix docs say:
- event_ast.rs:9-10: "…classified (not panicked on) via `decode_detailed`'s `DecodeError` — exactly the
  torn-vs-corrupt split `log_store` recovery needs." → over-credits the codec's error TYPE for the split.
- lib.rs:15-16: "`decode`'s error keeps the torn-vs-corrupt split recovery needs." → same.
Both are misleading: the codec's error only signals "this complete frame is bad" (→ Corrupt); the TORN
side is the framing layer's short-read detection, which never calls decode. Fix: reword both to credit the
framing layer for torn detection and describe the codec error as "a complete frame that fails to decode →
Corrupt" (not "the DecodeError drives the split"). Doc-only; the code is correct.

Owner: **v-agent-harness** (`cdz-kernel`: event_ast.rs, lib.rs). Reword so the torn-vs-corrupt split is
attributed to `log_store::decode_frames`' framing (short prefix/body → TornTail; complete-frame decode
`Err` → Corrupt), not to `decode`/`DecodeError`. This is the residual precision on the PR#1023 fix.
