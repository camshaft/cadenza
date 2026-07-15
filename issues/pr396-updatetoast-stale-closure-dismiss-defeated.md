# PR review comment — mirrored from GitHub PR #396 (Copilot inline)

- **PR:** #396 (MERGED)
- **File:** `guide/src/components/UpdateToast.tsx:59` (guard at :38, effect deps `[]` at :57)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3590624331
- **Link:** https://github.com/camshaft/cadenza/pull/396#discussion_r3590624331

## Comment (verbatim)
> The `useEffect` is intentionally mount-only (`[]`), but `check()` reads `updateAvailable` from the effect closure. That value will stay `false` forever, so polling won't actually stop after the toast is shown (and the "Dismiss" button will be ineffective because the next poll will set it back to true).

## Liaison triage — CONFIRMED against trunk (dev comment is WRONG)
Confirmed: `check()` guards with `if (cancelled || updateAvailable) return;`, but `updateAvailable` is
captured from the mount-only (`[]`) effect closure, so it's ALWAYS the initial `false`. The in-code
comment claims "a stale closure just means one extra fetch that no-ops" — that's incorrect. Two real
consequences:
1. Polling never stops after the toast is shown (a 5-min fetch keeps firing) — minor waste.
2. **Dismiss is defeated**: `✕` calls `setUpdateAvailable(false)`, but the next poll's `check()` still
   sees the stale `updateAvailable===false`, re-fetches, and (a newer version still deployed) calls
   `setUpdateAvailable(true)` again — the toast REAPPEARS after the reader dismisses it. That's a real
   UX bug, not the benign no-op the comment asserts.
Guide territory (v-guide). FIX: read `updateAvailable` via a ref (or gate the interval/dismiss on a ref
flag) so the guard sees the live value, or track a separate "dismissed" ref that suppresses re-show.
Fix on `trunk`. Quote + link in queue file.
