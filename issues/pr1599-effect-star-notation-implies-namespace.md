# PR #1599 review comment — implementation/design/DESIGN-host-capability-discovery.md (design-host-capabilities)

Mirrored from https://github.com/camshaft/cadenza/pull/1599 (PR: "design: control-plane return-channel
section — the I4 prerequisite, LOCKED with both harness owners").

## `effect/*` notation implies an `effect/` namespace the doc explicitly rejects (Copilot, :381, also :394, :427) — doc/clarity
> This section frames the split as `control/*` vs `effect/*` being a family-string prefix, but the very
> next bullet says effect families stay bare (`http`/`model`/...). Using `effect/*` here reads like an
> `effect/` namespace exists (or will exist) when the rest of the section explicitly rejects that; this
> could mislead implementors of the drive partitioning.

VERIFIED on the cand branch: the doc uses `effect/*` as shorthand for world-action effects (e.g. :385
"The Cedar authorizer stays a pure `effect/*` world-action gate", and the `Effect(...)  // effect/* :
authorize → …` registry comment) while :379 says effect families "are bare (existing, or a future bare
world-effect family)" and "Explicitly namespacing effect families later is a separate wire-migration
project; do NOT couple it here." So `effect/*` notation contradicts the stated bare-family reality — an
implementor could read it as an `effect/` prefix that must exist. Recurs at :394 and :427.

FIX: pick one convention — either (a) write "control-plane (`control/*`) vs world-effect (BARE family)"
and drop the `effect/*` glyph, or (b) add a one-line note "`effect/*` here is shorthand for
world-action effects, whose families are BARE — there is no `effect/` prefix". LOW/doc-clarity, but
worth it on a LOCKED design section others will implement against.
