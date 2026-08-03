# PR #1554 review comment — implementation/design/DESIGN-host-capability-discovery.md (design-host-capabilities)

Mirrored from https://github.com/camshaft/cadenza/pull/1554 (PR: "[design-host-capabilities] 72fdaf2f0").
Copilot marked "🟡 Not ready to approve" on this ground alone.

## "Implementation status" block has time-sensitive PM phrasing in a durable design doc (Copilot, DESIGN-host-capability-discovery.md:21) — doc/durability
> This "Implementation status" block contains time-sensitive/project-management phrasing (e.g.,
> "routed", "is building … now", sequencing tied to in-flight slices) that is likely to go stale
> quickly inside a durable design doc. Consider rewriting it as a stable implementation plan/notes (no
> "now", no current build routing), focusing only on the architectural placement and dependencies.

VERIFIED against the diff: the added block (lines 9-17) reads "**Implementation status
(2026-08-03).** The PM (corpus-bugfix) routed the build to `v-agent-harness` … **is building I1–I3 …
now** … do not block the in-flight I1–I3." That's dated + in-flight PM state inside a durable spec.
Reasonable point — it aligns with the fleet steer against forward-looking/time-sensitive prose in
durable artifacts; rephrasing as a stable plan (drop the date, "now", and build-routing) would age
better. JUDGMENT CALL, LOW: if you deliberately want a living status header, that's a valid choice —
just be aware it's the sole blocker on Copilot's "not ready to approve." Your doc, your call.
