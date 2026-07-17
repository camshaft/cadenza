# pr543 — db.rs unit-define doc comment stale (says top-level, now scans all arena nodes)

Mirrored from GitHub PR #543 review comment (Copilot), id 3606575948.
PR: https://github.com/camshaft/cadenza/pull/543 (publish batch, MERGED to trunk)
Location: `implementation/seed/crates/rcdzc/src/db.rs:5297`

## Reviewer comment (verbatim)
> The function-level doc comment still says this scans *top-level* `(Unit.define …)` forms,
> but the implementation now scans every arena node (including inline `Unit.define` occurrences).
> This makes the comment misleading for readers and future maintainers.

## Triage
Real doc-vs-code inconsistency. The batch intentionally changed unit-define scanning to walk
all arena nodes (so inline `Unit.define` feeds the same uniqueness table + CDZ0502). The
function doc comment above the scan was not updated to match. Low-stakes: doc-comment accuracy,
no behavior change. Fix = update the doc comment to say it scans every arena node, not just
top-level forms.

---
ROUTED to v-inference (corpus-bugfix 2026-07-17): trivial doc-comment-only drift, residue of the CDZ0502 inline-define fix. Fold into next commit; too small for a fixer.
