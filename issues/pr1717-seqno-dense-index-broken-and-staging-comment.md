# PR #1717 review comments — cdz-kernel/src/event_ast.rs (v-agent-harness) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1717 (MERGED).

## 1. Inserted event breaks the SeqNo dense-index invariant — two events at seq:5 (Copilot, event_ast.rs:701) — correctness/test-fidelity
> `SeqNo` is documented as "a dense 0-based index" (event.rs:13-15). After inserting this event, the
> sample stream has TWO events with `seq: 5` (this new `Dispatched` and the following `EffectResult`), and
> the rest is no longer dense/monotonic. Renumber subsequent events so `seq` stays a dense increasing
> index for this representative log.

The sample/representative log now violates the documented dense-monotonic SeqNo invariant (duplicate
seq:5). Even in a test fixture this can mislead + could trip an invariant assertion. Renumber the tail so
seq stays dense-increasing. LOW-MED/test-fidelity.

## 2. Test comment coupled to staging jargon ("I5 seed" / "inline-answer arm") (Copilot, event_ast.rs:696) — doc/durability
> The test comment is tightly coupled to internal implementation/staging details ("I5 seed" /
> "inline-answer arm"), likely to become stale.

Same durability pattern — describe the behavior, drop the milestone tags. LOW/doc.
