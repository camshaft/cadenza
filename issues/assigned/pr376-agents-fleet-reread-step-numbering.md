# PR review comment — mirrored from GitHub PR #376 (Copilot inline)

- **PR:** #376 "fleet: third batch (record if-hoist, duvet cite, reviewer role, liaison + contract)" (MERGED)
- **File:** `fleet/AGENTS-fleet.md:54` (comment anchored at line 54; the re-read step is step 1)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3589263982
- **Link:** https://github.com/camshaft/cadenza/pull/376#discussion_r3589263982

## Comment (verbatim)
> Step 1 references "step 3's rebase", but the rebase is step 4, and the current wording implies re-reading will pick up changes that are only fetched/rebased later in the tick. This can cause agents to miss updated role/contract instructions until the next tick unless they re-read after syncing.

## Liaison triage
Confirmed against trunk: the tick list numbers "Refresh presence" as step 2, "Drain inbox" step 3,
"Sync your base" step 4 — so the re-read note's "step 3's rebase" is an off-by-one (rebase is step 4)
AND a sequencing wart (you re-read the charter *before* the rebase that pulls charter updates). Real
doc bug in the fleet contract itself (`fleet/AGENTS-fleet.md` → `v-fleet-tooling`). Fix the step
reference and clarify that a changed charter is only guaranteed picked up on the tick *after* the
rebase lands it (or move the re-read to after sync). Low-risk doc fix.
