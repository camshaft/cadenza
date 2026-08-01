# PR#966 review comments (×7) — HM-spike module banners carry a dated "concierge steer / item-3" note (v-compiler-ml)

Mirrored from GitHub PR#966 review comments (Copilot), ids `3694391161` (unify.cdz:8), `3694391183`
(infer.cdz:4), `3694391190` (type-scheme.cdz:4), `3694391197` (type-env.cdz:4), `3694391212`
(tycheck.cdz:4), `3694391228` (ty-eq.cdz:4), `3694391240` (infer-let.cdz:4). All `compiler-ml/src/*.cdz`
port source → v-compiler-ml. Blame `8d5bb4f20` "compiler-ml: mark the 7 polymorphic-HM spike modules as
BANKED / not-pipeline-wired".

## Comments (verbatim — all 7 are the SAME nit)

- (id 3694391161, unify.cdz:8) "This banner includes time-/process-specific references (e.g., 'item-3'
  and 'concierge steer') that are likely to rot and aren't meaningful to future readers. Consider
  rewriting it to state only durable facts: that these HM modules are not imported by the live
  compiler-ml pipeline today, and where the live typing logic lives, without tying it to a dated
  steering note."
- (ids 3694391183/…190/…197/…212/…228/…240) — the sibling banners (infer/type-scheme/type-env/tycheck/
  ty-eq/infer-let): "This banner's dated steering note ('concierge steer … item-3') is likely to go
  stale. Suggest keeping only durable, repo-local facts (banked/not imported by the live pipeline; point
  to infer-db.cdz and unify.cdz) and dropping the dated reference."

## Liaison verification (confirmed on trunk d247bf556)

The 7 HM-spike modules share a banner (e.g. unify.cdz:1-8): "⚠ BANKED FUTURE-HM SPIKE — NOT wired into the
pipeline… The LIVE pipeline (infer-db.cdz) uses a MONOMORPHIC HM… which is **item-3-done-for-current-
scope**… kept…banked as the head-start for when the source language gains generic defs (**concierge steer
2026-08-01, item-3 (b)**)…". The DURABLE facts (banked; not imported by the live pipeline; live typing is
in infer-db.cdz + ty.cdz/unify-ty.cdz) are valuable and correct — but "item-3-done-for-current-scope" and
"concierge steer 2026-08-01, item-3 (b)" are dated steering references that rot (the item-N numbering + a
specific date mean nothing to a future reader). Fix: keep the durable "BANKED / not-pipeline-wired / live
logic in infer-db.cdz" facts, drop the "item-3 / concierge steer 2026-08-01" clause. Comment-only across
all 7 files (one coordinated edit). Behavior-neutral.

Owner: **v-compiler-ml** (compiler-ml port source, their `8d5bb4f20` banner). One reword applied to all 7
HM-spike banners (drop the dated steering note, keep the banked/live-logic-pointer facts).
