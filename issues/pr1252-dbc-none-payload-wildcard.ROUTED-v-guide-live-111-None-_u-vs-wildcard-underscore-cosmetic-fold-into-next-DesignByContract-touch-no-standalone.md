# PR #1252 review comment — guide/src/content/chapters/DesignByContract.tsx (v-guide)

Mirrored from https://github.com/camshaft/cadenza/pull/1252 (PR: "cand: v-guide — 9a17f63c6").

## `None` payload binds `_u` instead of wildcard `(_)` — inconsistent with other chapters (Copilot, DesignByContract.tsx:111) — doc/style nit
> For consistency with other guide examples, use the wildcard pattern `(_)` for the `None` payload
> rather than binding it as `_u` (most chapters use `((None _) ...)`). This keeps the example focused
> on the postcondition logic instead of introducing an unused binding name.

Low-priority consistency polish: match the `((None _) …)` wildcard form other chapters use, so the
example doesn't introduce an unused `_u` binding that distracts from the postcondition logic.
