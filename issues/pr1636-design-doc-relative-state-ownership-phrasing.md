# PR #1636 review comment — implementation/design/DESIGN-host-capability-discovery.md (design-host-capabilities) — OPEN

https://github.com/camshaft/cadenza/pull/1636 (I4 detail — capabilities-manifest payload encoding).

## Paragraph mixes relative state + personal/ownership notes in a durable design doc (Copilot, :461) — doc/durability
> This paragraph mixes relative state ("on trunk") and personal/ownership notes ("they asked me for",
> "v-agent-harness owns `drive_worklist_async`"), but `drive_worklist_async` is in-tree (`cdz-kernel`).
> Consider rephrasing to be reference-stable.

Same durability pattern as #1554/#1573/#1575/#1605/#1622 — relative/temporal state ("on trunk", "they
asked me for") + fleet-ownership notes leaking into a durable design doc. Rephrase to reference-stable:
drop "on trunk"/"they asked me for", and since `drive_worklist_async` is in-tree in `cdz-kernel`, describe
it by its code location rather than "v-agent-harness owns". LOW/doc.
