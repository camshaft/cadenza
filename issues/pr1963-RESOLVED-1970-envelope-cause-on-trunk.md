# PR #1963 review — cdz-kernel/src/kernel.rs (v-agent-harness) — MERGED — test-precision [VERIFIED]

https://github.com/camshaft/cadenza/pull/1963 (pin the FoldFailed error-capture). Copilot (id 3709732937)
flags the test asserts the body `caused_event` but NOT the envelope `Event.cause` edge.

## the FoldFailed test asserts `body.caused_event == inbound_hash` but never asserts `Event.cause` (the real causal-DAG edge) is set (Copilot, kernel.rs:1555) — test-precision [VERIFIED]
> This test asserts the `FoldFailed { caused_event }` field points at the inbound, but it doesn't assert
> the *envelope* `Event.cause` edge is also set. Since `Event.cause` is the actual causal-DAG linkage, the
> test can pass even if the kernel stops cause-linking `FoldFailed` events while still filling
> `caused_event` in the body.

VERIFIED. There are TWO distinct fields: `Event.cause: Option<Hash>` (event.rs:251 — the envelope causal
-parent edge, "what `cause` edges and the log's tamper-evidence point at") and `EventBody::FoldFailed {
caused_event }` (a body payload field). The test (kernel.rs:1550-1564) does `.find_map(|e| match &e.body {
FoldFailed { caused_event, .. } => Some(*caused_event) …})` then `assert_eq!(caused, inbound_hash)` — it
reads ONLY the body field. It never inspects `e.cause` on the FoldFailed event. So if the kernel regressed
to stop populating the envelope `Event.cause` for FoldFailed (while still filling the body's
`caused_event`), the test stays green — yet the causal DAG (which consumers/replay/tamper-evidence walk via
`Event.cause`, not the body field) would be broken. LOW-MED/test-precision — the pin under-specifies the
very linkage it's meant to protect.

Fix per Copilot: additionally assert the envelope edge on the FoldFailed event —
`let ff = s.log().iter().find(|e| matches!(&e.body, EventBody::FoldFailed{..})).unwrap();
assert_eq!(ff.cause, Some(inbound_hash), "FoldFailed's envelope cause edge links to the inbound")`. That
pins BOTH the body field AND the DAG edge, so a regression in either fails. v-agent-harness owns
cdz-kernel/src.
