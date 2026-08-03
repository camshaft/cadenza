# PR#1023 — cdz-kernel: docs stale after the event_ast codec swap actually landed (v-agent-harness)

Three Copilot review comments, all `cdz-kernel` → v-agent-harness. PR#1023 (batch that switched
`log_store` to encode/decode through `event_ast`/`cadenza_ast::codec`) DID the swap, so three docs that
still describe it as a pending follow-up are now genuinely stale. Gate = cdz-kernel own `cargo test`+clippy.

## Comment 1 (verbatim) — log_store.rs:18 (id 3696321730) — torn-vs-corrupt doc contradicts code

- "The module docs say the reader treats 'a body the codec rejects' as torn-tail tolerance, but the
  implementation classifies any decode failure on a complete frame as `RecoveryKind::Corrupt` (see
  `decode_frames`, which returns `Corrupt` on `Err(_)`). Update the docs to match the actual
  torn-vs-corrupt behavior (short prefix/body => TornTail; decode failure on a complete frame => Corrupt)."

### Liaison verification (confirmed on trunk 81e0f587b)

log_store.rs:14-16 (the torn-write-tolerance para): "…stops cleanly at the first frame it can't fully read
(short length prefix, short body, **or a body the codec rejects**) and returns every whole event before
it — the log survives a torn tail". But `decode_frames` (log_store.rs:198-213) does `match
event_ast::decode(body) { … Err(_) => return Recovered { kind: RecoveryKind::Corrupt, … } }` — a
complete-but-undecodable frame is `Corrupt`, NOT a torn-tail stop. The very next doc sentence (:17-18)
already says so correctly ("A frame that is *complete on disk but internally corrupt* … is surfaced as an
error"). So :16's "a body the codec rejects" clause is in the wrong list — it belongs to Corrupt, not the
"stops cleanly / survives a torn tail" clause. Fix: drop "or a body the codec rejects" from the torn-tail
sentence (a body-decode failure on a complete frame is Corrupt, already documented at :17-18). Doc-only.

## Comment 2 (verbatim) — lib.rs:16 (id 3696321747) — module map still says swap is a follow-up

- "The crate-level module map still says 'the `log_store` swap is a follow-up', but this PR already
  switches `log_store` to use `event_ast` for encoding/decoding. Please update this bullet so it reflects
  the current state (mapping + log_store integration are now both present)."

### Liaison verification (confirmed on trunk 81e0f587b)

lib.rs:16 (event_ast bullet): "(Mapping landed; the `log_store` swap is a follow-up.)" — FALSE now:
log_store.rs:102 calls `event_ast::encode(event)` and :198 calls `event_ast::decode(body)`. The swap
landed in this PR. Fix: update the bullet to say the mapping + log_store integration are both present
(log_store now encodes/decodes through event_ast, keeping its own u32 framing). Doc-only.

## Comment 3 (verbatim) — event_ast.rs:10 (id 3696321760) — "de-risks that swap" / "does not yet replace"

- "This module's docs still describe the `log_store` codec swap as a follow-up ('de-risks that swap'),
  but in this PR `log_store` already uses `event_ast::encode/decode`. Please revise the docs to reflect
  that `log_store` now consumes this mapping while still keeping its u32 framing."

### Liaison verification (confirmed on trunk 81e0f587b)

event_ast.rs:6-10: "This is the MAPPING layer only — it **does not yet replace** `log_store`'s framing
(that swap … is the follow-up slice). Building it in isolation … **de-risks that swap**". Same stale
premise as #2: log_store.rs:102/:198 now consume `event_ast::encode/decode`. Fix: revise to state
log_store now uses this mapping (keeping its u32 framing), rather than framing it as an un-done follow-up.
Doc-only.

Owner: **v-agent-harness** (`cdz-kernel`: log_store.rs, lib.rs, event_ast.rs). All three are doc-only
staleness introduced BY PR#1023's own swap — the code is correct; three doc sites still describe the
pre-swap world. #1 also mis-files the "codec rejects → Corrupt" case under the torn-tail clause.
