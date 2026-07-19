# pr556 — NotebookPage.tsx: two async-render out-of-order races (2 Copilot comments)

Mirrored from GitHub PR #556 review comments (Copilot).
PR: https://github.com/camshaft/cadenza/pull/556 (13-MR publish batch)
File: `guide/src/notebook/NotebookPage.tsx`

Both are stale-async-resolution races in the notebook UI: an in-flight `renderDocToSurface` can
resolve after newer state and clobber it.

## Comment 1 — id 3607332040 (NotebookPage.tsx:111) — surface-toggle effect misses committedDoc dep
> The surface-toggle re-render is async, but the effect only depends on `surface`. If `committedDoc`
> changes while a surface conversion is in flight (e.g. user edits during/after a toggle, or a late
> debounce commit lands), the old render can resolve and overwrite newer edits via
> `setDoc/setCommittedDoc`. Include `committedDoc`/`docSurface` in the effect deps so a new commit
> cancels the in-flight render (cleanup runs) and the latest doc is what gets rendered.

## Comment 2 — id 3607332050 (NotebookPage.tsx:159) — onSelectExample no stale-render guard
> `onSelectExample` starts an async `renderDocToSurface` but doesn't guard against out-of-order
> resolution. If a user selects example A then quickly selects example B, A's render can resolve
> later and overwrite the newer selection. Add a small `useRef` token to ignore stale renders.

## Triage
Both are real UI correctness bugs (last-write-wins races on async render) in the notebook example
picker / surface toggle — the picker that v-notebook just added. Copilot (accurate track record).
Owner = v-notebook (area=guide). Fixes: add deps + cleanup cancellation (#111); useRef stale-token
guard (#159).

---
RESOLVED (corpus-bugfix 2026-07-19, verified on trunk cac57fd66): BOTH async-render races fixed in
guide/src/notebook/NotebookPage.tsx via a monotonic `docRenderToken` (useRef, line 125), doc-comment cites
"(PR #556)".
• Comment 1 (surface-toggle effect, line 111→129): the toggle render bumps+captures the token; on resolution
  it commits ONLY if `!cancelled && docRenderToken.current === token` (line 134), and the effect cleanup sets
  `cancelled`. So a committedDoc change mid-toggle cancels the in-flight render — no clobber.
• Comment 2 (onSelectExample, line 159→216): also bumps+captures the token; on resolution `if
  (docRenderToken.current !== token) return; // a newer selection/toggle superseded this render` (line 227).
  Select-A-then-B no longer lets A's late render overwrite B.
The token is SHARED by both paths (toggle mid-example-switch also guarded); a separate `runToken` guards the
recompute chain. Exactly the reviewer's useRef stale-token ask. Owner (v-notebook) resolved — no corpus-bugfix action.
