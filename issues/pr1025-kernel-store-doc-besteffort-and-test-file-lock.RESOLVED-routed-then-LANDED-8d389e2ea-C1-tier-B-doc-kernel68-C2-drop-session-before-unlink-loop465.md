# PR#1025 — cdz-kernel: store-field doc overstates persistence guarantee + test deletes log while session holds it (v-agent-harness)

Two Copilot review comments, both `cdz-kernel` → v-agent-harness. Blame: batch #142 Session LogStore
write-through (the `attach_log`/`store`/`persist_error` tier-B durability slice). Gate = cdz-kernel own
`cargo test`+clippy.

## Comment 1 (verbatim) — kernel.rs store-field doc (id 3696362556), also :232

- "The `store` field docs claim that 'every appended event is persisted ... before `append` returns' and
  that S1 ordering is 'enforced IN-KERNEL'. However, `Session::append` always returns a hash even when
  `store.append(...)` fails (it only latches `persist_error`), so this guarantee only holds when
  persistence succeeds (i.e., when `take_persist_error()` is `None`). Please reword this to reflect the
  tier-B best-effort behavior and the error latch. This issue also appears on line 232 of the same file."

### Liaison verification (confirmed on trunk 0565a93e4)

kernel.rs:65-67 (`store` field doc): "…every appended event is persisted (append + flush) before `append`
returns — so the S1 'Dispatched durable before its effect routes' ordering is enforced IN-KERNEL". But
`append` (:245-…) does the write-through under `if self.persist_error.is_none()` and, on `store.append`
error, LATCHES it into `persist_error` (first-error-wins), skips further writes, and RETURNS THE HASH
regardless — it does not propagate or abort. So the "every event is persisted before append returns"
guarantee is TIER-B BEST-EFFORT: it holds only while no persist error has latched; after a persist
failure the on-disk log stops at the last good frame while in-memory continues. The append-body doc block
(:238-244) DOES describe this correctly ("A persist failure is LATCHED into `persist_error`… rather than
propagated"), so the field doc (:65) + the :232-area doc are the ones that overstate it. Fix: reword the
field doc + :232 to say persistence is best-effort tier-B — persisted before `append` returns UNLESS a
prior persist error is latched (driver checks `take_persist_error()`; on failure the run's log is not
fully durable and recovery heals the tail). Doc-precision; code is correct (the latch is intended tier-B).

## Comment 2 (verbatim) — tests/loop_and_recovery.rs (id 3696362570) — Windows file-lock

- "The test tries to delete the temp log file while `session` still has an attached `LogStore` open. On
  Windows this typically prevents deletion and leaves temp files behind (the error is currently ignored).
  Drop the session (closing the file) before `remove_file`."

### Liaison verification (confirmed on trunk 0565a93e4)

`attached_log_persists_through_on_append_no_manual_mirroring` (loop_and_recovery.rs:416) attaches a
`LogStore` to `session` (:440) and, at the end, `let _ = std::fs::remove_file(&path)` while `session` (and
thus the attached `LogStore`'s open `File`) is STILL LIVE. On POSIX/Linux (CI + primary platform) an open
file unlinks fine, so the test passes + cleans up. On WINDOWS the open handle blocks deletion → the
`let _ =` swallows the error → temp file leaks. Same class as the PR#1023 log_store.rs and the earlier
rename-over-existing findings: real for Windows, non-issue on the Linux CI. Fix: `drop(session)` (closing
the store's file) BEFORE `remove_file`. Low-sev (test hygiene / Windows-only leak). NOTE: check the other
attach_log tests in this file for the same pattern (the :461 recovery test builds its store in an inner
`{}` block so its file is already dropped before remove — only the live-session ones need the fix).

Owner: **v-agent-harness** (`cdz-kernel`: kernel.rs doc, tests/loop_and_recovery.rs). #1 = reword the
store-field + :232 doc to tier-B best-effort + the persist_error latch (append-body doc at :238 already
says it right). #2 = drop the session before remove_file in the live-session attach_log test(s).
