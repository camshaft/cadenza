# pr543 — cdz-run trap_message doc stale (says "render as before", now emits full cause chain)

Mirrored from GitHub PR #543 review comment (Copilot), id 3606575981.
PR: https://github.com/camshaft/cadenza/pull/543 (publish batch, MERGED to trunk)
Location: `implementation/seed/crates/cdz-run/src/lib.rs:145`

## Reviewer comment (verbatim)
> `trap_message` now intentionally renders non-`Trap` errors with the full anyhow cause chain
> (`{e:#}`), but the function doc comment immediately above still says non-`Trap` errors
> "render as before". This makes the public behavior contract unclear and risks future
> regressions back to outer-message-only rendering.

## Triage
Real doc-vs-code inconsistency. The batch intentionally changed `trap_message` to render
non-`Trap` errors with the full anyhow cause chain (`{e:#}`) for better diagnostics, but the
doc comment above the fn still says "render as before". Low-stakes: doc-comment accuracy pins
the intended contract so a future edit doesn't regress to outer-message-only. Fix = update the
doc comment to state non-`Trap` errors render with the full cause chain.

---
ROUTED to v-cdz-tooling (corpus-bugfix 2026-07-17): trivial doc-comment-only drift. Fold into next commit; too small for a fixer.

---
RESOLVED (corpus-bugfix 2026-07-19, verified on trunk f14408d1c): the trap-message doc in cdz-run/src/lib.rs
(trap_message, ~247-259) now correctly describes rendering the "whole anyhow CAUSE CHAIN inline (`{e:#}`), not
just the outer message" for both the Trap-code path and a non-Trap error — matching the current full-cause-chain
behavior. The stale "as before" phrasing the reviewer flagged is GONE (grep finds no "as before"). Doc-only nit
resolved by a peer. No corpus-bugfix action.
