# PR #1687 review comments — cdz-kernel/src/kernel.rs (v-agent-harness) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1687 (MERGED — genesis-seed the capability manifest).

## 1. `seed_capabilities_async` doc says "call ONCE after genesis" but nothing enforces it (Copilot, kernel.rs:394) — correctness/contract
> Documented as "Call this ONCE, immediately after genesis", but the method doesn't enforce/guard that.
> A second call appends another `control/capabilities` dispatch/result; calling it later cause-links the
> seed to the current tip rather than genesis (contradicting the doc). Making it idempotent (and
> asserting the precondition in debug) would prevent double-seeding and keep cause-linking honest.

The seed-once contract is doc-only; a double call silently double-seeds + mis-cause-links (to tip, not
genesis). Guard it: make idempotent (no-op if already seeded) or `debug_assert!` the first-call
precondition (e.g. event-log is at genesis). MED — a mis-seed corrupts the capability manifest's causal
provenance. Fix-forward.

## 2. Doc comment very long with milestone jargon + emphatic/history narration (Copilot, kernel.rs:377) — doc/durability
> The new doc comment is extremely long and includes internal milestone jargon ("I5/I4b"), emphatic
> phrasing ("BORN KNOWING", "ALWAYS"), and implementation-history narration.

Same durability pattern (#1554/#1622/#1664/#1687-family). Trim to the stable contract; drop the milestone
tags + emphatic history. LOW/doc.
