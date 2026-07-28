# PR#841 review comment — --reap-dead-letters mutates inboxes gated on status=stopped ONLY, not the documented stop-file AND

Mirrored from GitHub PR review comment (Copilot), id `3646362309`.
PR: https://github.com/camshaft/cadenza/pull/841 (batch-staging; fix belongs on trunk)
Location: `xtask/src/fleet.rs:3043` (the `reap_dead_letters` loop), landed `fdc7a6ff5`.

## Comment (verbatim)

> `--reap-dead-letters` is documented as only mutating an agent's inbox when it is BOTH `status=stopped`
> and has a stop-file, but this loop reaps based solely on the `stranded` list (which is derived from
> `status == "stopped"` only). If the registry is out of sync (stopped without stop-file), this could
> mutate an inbox for an agent whose loop hasn't demonstrably exited.

## Liaison verification (CONFIRMED on trunk — real doc-vs-code gap w/ safety implication)

- The flag doc (fleet.rs:695): "Only reaps an agent that is BOTH registry status=stopped AND has a
  stop-file (its loop demonstrably exited — never a merely-idle live agent)."
- `reap_stranded_dead_letters`'s own doc (fleet.rs:1919): "Callers gate on stop-file + status=stopped,
  so this only ever touches a demonstrably-exited agent's inbox."
- BUT the reap loop (fleet.rs:3026 `for (name, _, _) in &stranded`) iterates `stranded` =
  `find_stranded_stopped_inboxes` (fleet.rs:1896), which filters on `a.status == "stopped"` ONLY — no
  stop-file existence check. So the documented AND-stop-file gate is NOT actually applied by the caller.

Impact: if the registry is out of sync (status=stopped but the stop-file is absent — e.g. a crash/kill
that never dropped it, or a re-activated agent whose stop-file was cleared but status not yet updated),
`--reap-dead-letters` would mutate (reap→processed/) the inbox of an agent whose loop has NOT
demonstrably exited. It's LOSSLESS (every reap is logged to dead-letters.log first), so nothing is
destroyed, but it violates the stated "demonstrably-exited only" safety contract and could steal a live
(if mis-registered) agent's un-drained mail.

Fix (per Copilot): add the stop-file existence check to the gate — either filter `find_stranded_
stopped_inboxes` on `fleet.stopfile(&a.name).exists()`, or check it in the reap loop before
`reap_stranded_dead_letters`. (Then the code matches both docstrings.) Owner: v-fleet-tooling (single
owner of `fleet.rs`; commit `fdc7a6ff5`). Routed as a note flagged SAFETY (guarded by the lossless log,
but the contract is currently unenforced).
