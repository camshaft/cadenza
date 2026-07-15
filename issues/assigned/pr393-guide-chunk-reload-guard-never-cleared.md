# PR review comment — mirrored from GitHub PR #393 (Copilot inline)

- **PR:** #393 (MERGED)
- **File:** `guide/src/main.tsx:6` (root cause in `guide/src/components/chunkError.ts`)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3590413291
- **Link:** https://github.com/camshaft/cadenza/pull/393#discussion_r3590413291

## Comment (verbatim)
> `RouteError` uses a sessionStorage one-shot guard (`cadenza:chunk-reload-attempted`) to prevent reload loops, but nothing clears it after a successful load. That makes auto-reload permanently disabled for the rest of the tab session after the first stale-deploy incident, which contradicts the intent in `chunkError.ts` (re-arm after success). Clearing the guard at startup keeps the loop protection while allowing a later stale deploy to auto-reload again.

## Liaison triage — CONFIRMED against trunk
Confirmed: `chunkError.ts` defines `clearAutoReloadGuard`, documented "call after a route/chapter loads
SUCCESSFULLY, so a future stale-deploy navigation can auto-reload again (the guard is only meant to
break a same-session reload LOOP)". But `git grep` finds NO production caller of `clearAutoReloadGuard`
in `guide/src` (only its own definition + tests). So once `shouldAutoReload` sets the guard on the first
chunk-load failure, it's never cleared, and auto-reload stays disabled for the rest of the tab session —
exactly contradicting the documented re-arm intent. Guide territory (v-guide). FIX: call
`clearAutoReloadGuard(sessionStorage)` on a successful app/route load (e.g. at startup in main.tsx or on
a successful route render). Fix on `trunk`. Quote + link in queue file.
