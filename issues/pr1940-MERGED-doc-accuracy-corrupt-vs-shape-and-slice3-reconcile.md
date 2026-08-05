# PR #1940 review — cdz-kernel + design doc (v-agent-harness) — MERGED — doc-accuracy [VERIFIED]

https://github.com/camshaft/cadenza/pull/1940 — MERGED 2026-08-04T04:05:19Z (§6a Error resilience &
self-heal doc). Copilot 3 inline; two fresh doc-accuracy points + one DUPLICATE of my #1938 cluster.

## `event_ast.rs:770` doc says an unknown close-outcome head is `Corrupt`, but `read_close_outcome` returns `EventAstError::Shape` (Copilot id 3709335845) — doc-accuracy [VERIFIED, LOW]
> This doc says an unknown close-outcome head is `Corrupt`, but `read_close_outcome` returns
> `EventAstError::Shape` (which the framing layer may treat as corruption, but it's not a distinct
> `Corrupt` variant here). Update the comment to match the actual error classification used by this module.
Doc-accuracy on a merged doc/comment. LOW. Reword to `Shape` (or note the framing layer maps Shape→corrupt).

## `design/agent-harness-kernel.md:294 & :309` — §6 says `close` AUTO-delivers `child-completed(...)` to the parent, but §6a marks that auto-delivery as slice-3 (pending): reads as both "implemented" and "not yet" (Copilot id 3709335855) — doc-consistency [VERIFIED, LOW]
> §6's bullet says `close` auto-delivers `child-completed(...)` to the parent, but §6a's incremental path
> marks that auto-delivery as slice-3 (pending). This reads as "already implemented" in one place and "not
> yet implemented" in another; please reconcile by making §6's wording explicitly slice-3/future-tense (or
> updating the slice status if it's already built).
Internal contradiction in the design doc. LOW/clarity — reconcile §6's tense to slice-3/future, or bump
the slice status if built. (Per the locked-design-doc batching rule, this is a real consistency fix, not
pure cosmetics — worth relaying, but batchable with the doc reword below.)

## `event.rs:217` (+ :433) doc claims "no wire break"/tolerant but decoder rejects unknown tags (Copilot id 3709335864) — DUPLICATE of #1938 cluster (comment 3709301191)
> The `CloseOutcome` doc comment claims additive growth "with no wire break" … but the current v0 binary
> decoder rejects unknown variant tags (`DecodeError::BadTag`). …adjust so it doesn't imply
> forward-compatibility guarantees the implementation does not provide.
Already filed on #1938 (comment 3709301191, same file:line 217) and routed to v-agent-harness as part of
the wire-break cluster. NOT re-filing — cross-referenced. The fix is the same: reword after the
Success-legacy/Failure-new-tag compat lands. (Copilot re-flagged it on the §6a doc PR that touched
adjacent lines; :433 is the decode-site companion of the same claim.)
