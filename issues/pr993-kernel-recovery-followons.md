# PR#993 review comments — cdz-kernel PR#990-fix follow-ons: recover drops `corrupt`, stale "both paths" comment, Cursor::len doc, Recovered→enum (v-agent-harness)

Mirrored from GitHub PR#993 review comments (Copilot), ids `3695378138` (log_store.rs:47), `3695378141`
(kernel.rs:475), `3695378145` (event.rs:213, +703), `3695380903` (log_store.rs:43). All
`implementation/seed/crates/cdz-kernel/*` → v-agent-harness. Blame `42a86b872` "fix(cdz-kernel): 4 PR#990
review findings" — these are RESIDUAL follow-ons on that fix (the PR#990 4 findings themselves are DONE).

## Comments (verbatim)

- (id 3695378138, log_store.rs:47): "`Recovered` now distinguishes clean EOF vs torn tail vs corruption
  via `corrupt`, but the public one-call recovery entry point (`Session::recover`) currently drops this
  flag and only returns `torn_tail`/`open_effects`. Callers using `Session::recover` therefore still
  can't alarm/hard-fail on corruption as described here. Consider propagating `corrupt` in
  `RecoveryReport` or turning it into a dedicated `RecoverError` variant."
- (id 3695378141, kernel.rs:475): "The doc comment says `observable` is the 'single source of truth used
  by BOTH the live `drive` path and `replay`', but only `replay` consults this helper; the live path
  folds by calling `fold_tip` at specific append sites. As written, the comment is misleading…"
- (id 3695378145, event.rs:213, +703): "The `Cursor::len` doc comment implies oversized lengths become
  `Truncated` once they fit in `usize`, but very large values can also become `BadLength` due to overflow
  checks in `take` (e.g., when `pos + len` overflows). Updating the comment would make the error behavior
  clearer…"
- (id 3695380903, log_store.rs:43): "If it's mutually exclusive then it should be an enum" (re
  `torn_tail`/`corrupt` being two bools documented as mutually exclusive).

### Liaison verification (confirmed on trunk 18dba958f; blame `42a86b872`)

1. log_store.rs:47 — the PR#990 corruption-≠-EOF fix added `Recovered.corrupt`, but the reviewer notes
   the PUBLIC `Session::recover` entry point drops it (returns only `torn_tail`/`open_effects`) — so
   external callers STILL can't alarm on corruption, which is the whole point. Propagate `corrupt` into
   `RecoveryReport` (or a `RecoverError::Corrupt` variant). This is the "half-fixed" tail of my PR#990
   corruption route — the flag exists internally but doesn't reach callers. Worth doing.
2. kernel.rs:475 — the PR#990 replay-equiv fix's `observable` helper doc says "single source of truth for
   BOTH drive and replay", but only `replay` calls it; `drive` folds via `fold_tip` at append sites.
   Stale/misleading (a future edit might assume drive is gated by `observable`). Reword.
3. event.rs:213 (+703) — `Cursor::len` doc implies oversized→`Truncated`, but `take`'s `pos+len` overflow
   check can also yield `BadLength`. Doc-precision on the untrusted-length fix.
4. log_store.rs:43 — `torn_tail`+`corrupt` are two bools documented "mutually exclusive"; an enum
   (`RecoveryKind::{Clean, TornTail, Corrupt}`) makes the exclusivity type-enforced instead of a
   doc-promise (a follow-through on comment 1's "dedicated variant" idea). Design call.

Comment 1 is the substantive one (the corruption alarm doesn't reach `Session::recover` callers); 2-4 are
doc/API-shape follow-ons. All in the PR#990-fix's own code.

Owner: **v-agent-harness** (`cdz-kernel`; `42a86b872`, the PR#990 fix). Propagate `corrupt` to
`Session::recover`/`RecoveryReport`; reword the "both paths" `observable` comment; clarify `Cursor::len`
BadLength path; consider a `Recovered`-kind enum. Gate = cdz-kernel's own `cargo test`+clippy.
