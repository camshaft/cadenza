# PR#934 review comment — ClearStray exits on NotFound (TOCTOU); should treat NotFound as success (v-fleet-tooling)

Mirrored from GitHub PR#934 review comment (Copilot), id `3685524761`.
File: `xtask/src/fleet.rs:5434` — v-fleet-tooling. Blame `5f4555ac9` "fleet inbox --processed: don't
rename over an existing archived copy (crash-safe (true,true) → clear stray)" — the very PR#929 fix I
routed; this is a follow-on idempotency gap in it.

## Comment (verbatim)

- (id 3685524761, xtask/src/fleet.rs:5434) "`ConsumeAction::ClearStray` exits with an error if
  `remove_file(src)` returns `NotFound`, but that can happen if the live copy disappears between the
  earlier `src.exists()` check and the removal attempt (e.g., another drain/archive raced). Since the
  goal state is 'not in the live inbox', `NotFound` should be treated as a successful no-op to preserve
  the intended idempotent/crash-safe behavior."

## Liaison verification (confirmed on trunk e8ed7b8c3)

The `ClearStray` arm (fleet.rs:5426-5440): `if let Err(e) = std::fs::remove_file(&src) { eprintln!(…);
std::process::exit(1); }`. Any error — INCLUDING `io::ErrorKind::NotFound` — exits 1. But `ClearStray` was
selected from an EARLIER `(src_exists, dst_exists)` = `(true, true)` probe; between that probe and this
`remove_file`, a raced concurrent drain/archive (or a re-run) can remove `src`, so `remove_file` returns
`NotFound`. The GOAL STATE of ClearStray is "the live inbox no longer holds `msg`" — which `NotFound`
ALREADY satisfies. Exiting 1 on it turns a benign race into a spurious failure, undermining the exact
idempotent/crash-safe intent this arm was added for (PR#929). Fix (Copilot's, sound): treat
`Err(e) if e.kind() == NotFound` as SUCCESS (the stray is already gone → proceed to re-list); only a
REAL error (permission, IO) should exit 1. Robustness/idempotency.

Owner: **v-fleet-tooling** (`xtask/src/fleet.rs` inbox resolver, `5f4555ac9` — the PR#929 fix's own
follow-on). Match `NotFound` as a no-op success in the ClearStray remove.
