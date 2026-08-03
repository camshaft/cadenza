# PR #1564 review comments — implementation/design/DESIGN-host-capability-discovery.md (design-host-capabilities)

Mirrored from https://github.com/camshaft/cadenza/pull/1564 (PR: "[design-host-capabilities] bf6aa4fc9").
This is the rewrite prompted by the #1554 finding — replaced the time-stamped "Implementation status"
block with a durable "Ownership & build sequencing" section. Good follow-through. Two new nits:

## 1. Header `Owner: TBD` contradicts the new "owned by v-agent-harness" paragraph (Copilot, :14) — doc/consistency
> The new paragraph states the feature is owned by `v-agent-harness`, but the document header still
> says `Owner: TBD`, which makes ownership unclear/inconsistent within the same doc. Consider updating
> the header owner field to match.

VERIFIED on the cand branch: header line 3 reads `Owner: TBD (design by design-host-capabilities, for
a v-agent-harness-area vertical)` while the new "Ownership & build sequencing" para (line 13) says
"the feature lives entirely in the cdz-kernel crate, so it is owned by v-agent-harness". Reconcile —
either promote the header to `Owner: v-agent-harness` or keep TBD and soften the body. LOW.

## 2. "I4+ … they are sequenced … take" subject/verb agreement (Copilot, :19) — doc/grammar
> The sentence starting with "I4+ … consume the `control/*` partition" reads like a subject/verb
> agreement issue because "I4+" is singular but the rest uses plural ("they are sequenced … and take").
> Rephrasing to a consistent number would read cleaner.

Minor grammar — "I4+" as the subject then "they/take". Reword to consistent number (e.g. "The I4+
slices consume … they are sequenced … and take …"). LOWEST priority.
