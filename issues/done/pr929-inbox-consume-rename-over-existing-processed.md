# PR#929 review comment — inbox_consume Move on (true,true) renames over an existing processed/ copy (v-fleet-tooling)

Mirrored from GitHub PR#929 review comment (Copilot), id `3685128540` (:5371, also :5400, :8398).
File: `xtask/src/fleet.rs` — v-fleet-tooling. Blame `9ebf64dcd` "fleet inbox: add `--processed <msg>` so
the resolver owns the archive path on BOTH sides (kills the move-step drain-stall)".

## Comment (verbatim)

- (id 3685128540, xtask/src/fleet.rs:5371) "`inbox_consume_action` currently returns `Move` when the
  message exists in both the live inbox and `processed/` (`(true, true)`). In `inbox_consume`, that flows
  into `std::fs::rename(src, dst)` without handling an existing destination, which can overwrite the
  archived copy or fail (platform-dependent), undermining the intended idempotent/mid-move-crash
  behavior. Treat `(true, true)` as already archived and let `inbox_consume` clean up any stray live
  copy. This issue also appears in the following locations of the same file: line 5400, line 8398."

## Liaison verification (confirmed on trunk d5df868bc)

`inbox_consume_action` (fleet.rs:5365-5371):
```
match (src_exists, dst_exists) {
    (true, _) => ConsumeAction::Move,
    (false, true) => ConsumeAction::AlreadyDone,
    (false, false) => ConsumeAction::Missing,
}
```
The `(true, _)` arm returns `Move` even when `dst_exists` is TRUE — i.e. the message is in BOTH the live
inbox AND `processed/` (a mid-move-crash / re-run / double-delivery). `inbox_consume` then does
`std::fs::rename(src, dst)` with `dst` already present → on POSIX it OVERWRITES the archived copy (loses
the canonical processed version if they differ), on Windows it FAILS — either way undermining the
idempotent / crash-safe intent this `--processed` resolver was added for (same rename-over-existing class
as PR#903's provider cache). Fix (Copilot's, sound): treat `(true, true)` as ALREADY-ARCHIVED — return a
variant that has `inbox_consume` just REMOVE the stray live `src` (the processed copy is authoritative),
not rename over the existing dst. The `:5400`/`:8398` sites are flagged same-class (the rename call +
another consumer — verify each handles the existing-dst case).

Owner: **v-fleet-tooling** (`xtask/src/fleet.rs` inbox resolver, `9ebf64dcd`). Add a `(true,true)` =
already-archived arm (remove stray live copy, don't rename over dst); check :5400/:8398.
