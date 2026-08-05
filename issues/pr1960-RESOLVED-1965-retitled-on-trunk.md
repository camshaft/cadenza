# PR #1960 review — design/agent-harness-kernel.md (v-agent-harness) — MERGED — doc-clarity [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/1960 (reconcile §6 supervision-tree build-status — the
follow-through on my #1940 §6/§6a slice-3 reconciliation). Copilot (id 3709647022) flags a residual
tense/status ambiguity.

## the "Scenario (works fully within this)" heading now reads as a claim about the CURRENT kernel, though spawn/child-completed (slices 2–3) aren't implemented (Copilot, agent-harness-kernel.md:291 & :356) — doc-clarity [VERIFIED, LOW]
> The new build-status paragraph clarifies that `spawn` and `child-completed` auto-delivery (slices 2–3)
> are not implemented yet, but the later heading "Scenario (works fully within this)" now reads like a
> claim about the current kernel. Tweaking that heading to make it explicitly "intended/planned" would
> avoid confusion.

VERIFIED-plausible on the doc's own terms: #1960 added a build-status paragraph that (per my #1940
reconciliation) correctly marks spawn + child-completed auto-delivery as slices 2–3 / not-yet-built, but
the downstream "Scenario (works fully within this)" heading (appears at :291 and :356) still reads in the
present tense, so a reader hitting the heading without the status paragraph fresh in mind takes it as a
current-kernel guarantee. Consistency nit — the exact class my #1940 §6/§6a finding was about (one place
future-tense, another present). LOW/doc-clarity. Fix per Copilot: retitle to "Scenario (intended /
planned — slices 2–3)" or similar so the heading carries the same not-yet-built framing as the status
paragraph. Batchable with any other agent-harness-kernel.md prose touch. v-agent-harness owns the design
doc.
