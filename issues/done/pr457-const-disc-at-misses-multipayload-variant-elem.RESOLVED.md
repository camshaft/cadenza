# PR review comment — mirrored from GitHub PR #457 (Copilot inline)

- **PR:** #457 "fleet: seventy-seventh batch (nested-match MISCOMPILE fix, …)" (OPEN at triage; file on trunk)
- **File:** `implementation/seed/crates/rcdzc/src/backend/wasm/select.rs:5275` (`const_disc_at`)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3593308688
- **Link:** https://github.com/camshaft/cadenza/pull/457#discussion_r3593308688

## Comment (verbatim)
> `const_disc_at` claims to walk `Payload`/`Elem` steps through constant `SumNew`/`Tuple` cores, but it currently only handles `Payload` for `SumNew` when `payloads.len() == 1` and does not handle `Elem(i)` into a `SumNew`'s payload list (nor the multi-payload `Payload`+`Elem` encoding). That means constant scrutinee discriminants can still fail to be recovered on paths that pass through a multi-payload variant, falling back to variant 0 and risking the same "wrong payload depth" class of miscompile in other shapes. Consider mirroring the Rust backend's constant-path walker (consume `Payload` + following `Elem` for multi-payload variants, and allow `Elem` into `SumNew { payloads }`).

## Liaison triage — CONFIRMED against trunk
Confirmed in select.rs `const_disc_at`: the step-match handles `(Payload, SumNew { payloads })` and
`(Elem(i), Tuple/ListNew)`, but has NO `(Elem(i), SumNew { payloads })` arm — so a path stepping into a
MULTI-PAYLOAD variant's payload list (encoded as Payload + Elem) hits `_ => return None`, losing the
constant discriminant → the caller falls back to variant 0 → the "wrong payload depth" miscompile class
(this PR itself is a "nested-match MISCOMPILE fix", so the surrounding area is miscompile-prone). FIX
(as reviewer): mirror the Rust backend's constant-path walker — consume `Payload` + the following
`Elem` for multi-payload variants, and allow `Elem` into `SumNew { payloads }`. Wasm-backend
const-discriminant soundness → route to `corpus-bugfix` PM (repro: a match on a constant multi-payload
variant projected via Payload+Elem). Fix on `trunk`. Quote + link in queue file.

--- RESOLVED 2026-07-16 by v-memory-safety (mr 148b72d1f, pending merge) ---
Fixed all three fold walkers (fold_sum_path/const_at_path in lower.rs, const_disc_at in select.rs): a
Payload over a multi-payload SumNew is a no-op landing on the payload tuple, (Elem(i), SumNew{payloads})
selects payloads[i]. (Mk 7 (IB 42)) now folds to 742; runtime sibling already correct. Unit test + 2
corpus cases pinned. gate 3181/3/0, check green.
