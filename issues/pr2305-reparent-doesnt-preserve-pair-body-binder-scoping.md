# PR #2305 review — rcdzc/src/effects.rs (v-effects, cand/v-effects-53d218db7) — OPEN — binder-scoping correctness [VERIFIED-plausible, MED]

https://github.com/camshaft/cadenza/pull/2305 (re-anchor the two-hole refold's spliced continuation so a
body free-var resolves — the breaker pm-family false-CDZ0101 fix, banked NEXT per the effects live-state
line). Copilot 1 inline (id 3724510885, effects.rs:7963).

## the new `db.reparent(filled, Some(anchor), child_ix_of(handle_body))` does NOT preserve binder scoping when `handle_body` is the BODY position of a 2-element pair (`(pattern body)` match arm / `(name init)` let) — resolve helpers require the scope-walk `from` to be EXACTLY the pair's recorded `body` child (`pb[1]`), so a binder referenced from inside `filled` can still fail to resolve / spuriously decline (Copilot, effects.rs:7963) — binder-scoping [VERIFIED-plausible, MED]
> The new re-anchoring uses `db.reparent(filled, Some(anchor), child_ix_of(handle_body))`, but this does not
> preserve binder scoping when `handle_body` is the body position of a 2-element pair (e.g. a `(pattern
> body)` match arm or `(name init)` let binding). Name resolution helpers like `resolve::match_arm_binds`
> require the scope-walk's `from` node to be exactly the pair's recorded `body` child; with the current
> approach `from == filled` but `pb[1] == handle_body`, so pattern/let binders referenced from inside
> `filled` can still fail to resolve during the recursive fold (or cause spurious fold declines). Reuse the
> existing `reparent_under_handle_site` helper here so the pair-rebuild case is handled consistently.

VERIFIED the structural premise against source (rcdzc, PR head):
- `reparent_under_handle_site` EXISTS (effects.rs:3111) — the tail helper the diff's own comment says it
  "Mirrors."
- resolve.rs REQUIRES `from == pb[1]` for pair-body binder resolution: THREE explicit guards —
  `if from != pb[1] && Some(from) != guard_cond` (resolve.rs:1880, :1941) and
  `if pb.len() != 2 || from != pb[1]` (:2005), plus `match_arm_binds` (:2752) whose `(pattern, body) =
  (pb[0], pb[1])` walk is position-based. So the claim's mechanism is real: if the re-anchor leaves the
  recorded body child = `handle_body` (`pb[1]`) while the scope-walk enters from `filled`, a
  pattern/let-bound name referenced inside `filled` hits `from != pb[1]` → NOT bound → either fails to
  resolve or (given this fix targets false-CDZ0101) re-introduces exactly the spurious-decline class the PR
  is closing, just for the pair-body-position handle sub-case.

MED / binder-scoping correctness. RELAYED AS PLAUSIBLE, not certain: whether a handle BODY ever actually sits
in a 2-element pair's body position (`(pattern body)` / `(name init)`) during the recursive refold — vs.
always being a top-level or list-position body where the current `child_ix_of` anchor is fine — is v-effects'
eval/scope-semantics call. The fix is sound for the list-position case it targets; the open question is the
pair-body sub-case. Fix per Copilot: route the pair-rebuild through the existing `reparent_under_handle_site`
helper (or otherwise ensure the recorded body child, not just `filled`, is what the scope-walk enters from) so
pair-body handles resolve consistently. v-effects owns rcdzc effects. PR OPEN → foldable pre-merge. Worth a
pm-family pin with a handle body in `(pattern body)` position if v-effects confirms the sub-case is reachable.
