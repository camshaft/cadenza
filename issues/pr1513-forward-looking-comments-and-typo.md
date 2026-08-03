# PR #1513 review comments — cdz-kernel/src/{effect,event}.rs (v-agent-harness)

Mirrored from https://github.com/camshaft/cadenza/pull/1513 (PR: "[v-agent-harness] f6706790b").

## 1. `EffectRequest::new` doc overstates benefit + typo + stale jargon (Copilot, effect.rs:121) — doc
> The doc comment for `EffectRequest::new` is forward-looking and currently overstates the benefit:
> simply adding `new()` doesn't avoid downstream breakage until call sites migrate off struct
> literals. It also contains a typo ("reding") and some jargon that may go stale ("effect-schema
> arc", "slice 2"). Consider rewording to describe the present behavior and the intended (but
> conditional) migration benefit more precisely.

Fix the "reding" typo, drop/soften the "effect-schema arc"/"slice 2" forward-looking jargon, and
state the benefit as conditional (only realized once call sites migrate off struct literals).

## 2. `matches_family` comment references future "slice 2" / internal project name (Copilot, event.rs:58) — doc
> This comment references a future planned change ("slice 2") and a specific internal project name
> ("effect-schema arc"); that kind of forward-looking note can become stale and doesn't affect the
> behavior of `matches_family`. Consider rephrasing in terms of the current invariant: all
> family-keyed routing/authz should share this helper to avoid drift.

Reword to the present-tense invariant (all family-keyed routing/authz shares this helper) rather than
naming a future slice / internal arc. (Related to the #1507 event.rs matches_family doc note.)
