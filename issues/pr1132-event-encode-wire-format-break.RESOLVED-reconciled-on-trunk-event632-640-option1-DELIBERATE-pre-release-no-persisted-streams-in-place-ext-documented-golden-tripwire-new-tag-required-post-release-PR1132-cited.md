# PR #1132 review comment — cdz-kernel/src/event.rs (v-agent-harness) — OPERATOR TOP PRIORITY PR

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1132
(PR: "cand: v-agent-harness — (OPERATOR TOP PRIORITY)").

## ⚠ Adding `token` bytes to existing body tags 4/5/6 breaks the frozen canonical log wire format (Copilot, event.rs:318) — CORRECTNESS / format-compat
> `Event::encode`/`decode` is documented and tested as the canonical on-disk log format and as a
> concatenated stream (see "frozen — §16c-S3" and `decode_reports_offset_so_a_stream_can_be_walked`).
> Adding `token` bytes to existing body tags 4/5/6 changes the wire format for
> `TimerArmed`/`TimerFired`/`AuthzDenied` without any per-event framing or versioning, so previously
> persisted streams using the old encodings will not just fail to decode — they can DESYNCHRONIZE
> (the next event's seq bytes get consumed as the token's present-tag/len), making the remainder of
> the log unreadable/corrupt.
>
> To keep the canonical log format evolvable, consider either (a) introducing new body tags for the
> token-bearing encodings and keeping tags 4/5/6 as legacy tokenless encodings (new decoder accepts
> both), or (b) adding an explicit version/length framing layer before extending existing variants.

This is the important one this batch — it's on the OPERATOR TOP PRIORITY agent-harness PR and touches
the FROZEN §16c-S3 canonical log format. If any persisted stream predates this change, extending
tags 4/5/6 in place can corrupt the tail of an existing log (stream desync, not a clean decode
failure). Worth v-agent-harness confirming whether: (1) there are any persisted old-format streams to
be compatible with (if the format is pre-release / no durable logs exist yet, in-place extension may
be acceptable — but should be a DELIBERATE call, documented + the §16c-S3 "frozen" note updated); or
(2) it needs the new-tag or version-framing approach Copilot suggests. Either way the frozen-format
claim and the encoding should be reconciled.
