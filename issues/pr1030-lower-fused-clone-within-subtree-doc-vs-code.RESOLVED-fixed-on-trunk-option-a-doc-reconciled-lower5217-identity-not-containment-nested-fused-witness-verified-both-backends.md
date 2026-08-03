# PR#1030 — rcdzc lower.rs: fused-match arm-clone doc claims a "within the cloned subtree" copy case the code no longer implements (v-patterns)

One Copilot review comment (repeated at 3 lines: 5208, 5212, 5230), `implementation/seed/crates/rcdzc/src/lower.rs`
→ v-patterns (match-fusion arm-clone lowering). Blame `a5f7cfafb` "lower: fused-match arm-clone copies the
FUSED match's own binder, shares only an ENCLOSING capture (fixes rust Option/Result-pipeline regression
from 11c39b005)". Gate = `cargo test -p rcdzc` + `xtask gate` + (if emit changes) compiler-ml self-host.

## Comment (verbatim) — lower.rs:5208 (id 3696553895)

- "The doc comment says a fused-match binder is copied when its `SumPayload.scrutinee` is `fused_scrut`
  *or when the scrutinee is within the cloned subtree*, but the implementation only checks
  `fused_scrut == Some(scrutinee)`. Either restore the 'within clone' case (by threading a clone root
  like the previous implementation did) or update the docs to match the current behavior. This issue
  also appears in the following locations of the same file: line 5212, line 5230."

## Liaison verification (confirmed on trunk e68033e83)

The fn doc (:5207-5208) says a binder of the MATCH BEING FUSED is copied when "its `SumPayload.scrutinee`
IS `fused_scrut`, OR the scrutinee is within the cloned subtree itself". The enclosing-match case
(:5212) and the inline comment (:5229-5231) repeat the same "(or sits within the cloned subtree)" phrasing.
But the actual predicate (:5236-5237) is ONLY:
```
Resolved::SumPayload { scrutinee, .. } if fused_scrut == Some(scrutinee)
```
— there is no "within the cloned subtree" test; nothing threads a clone-root/subtree boundary. The PRIOR
impl (`11c39b005`, the immediately-preceding commit) DID thread a clone root; `a5f7cfafb` simplified the
classification to the single `fused_scrut == Some(scrutinee)` check. So the doc describes a second copy
condition ("within the cloned subtree") the code no longer implements. TWO possibilities, v-patterns' call:
- (a) the simplification is COMPLETE and correct — the `fused_scrut == Some(scrutinee)` check alone
  captures every own-binder of the fused match (the "within subtree" case is subsumed because a fused
  match's own binders all read `fused_scrut`) → then the doc's "or within the cloned subtree" is stale,
  DELETE it (3 sites); OR
- (b) there really IS a case where an own-binder's scrutinee ≠ `fused_scrut` but sits within the clone
  (e.g. a nested fused match inside the cloned arm) that SHOULD copy and currently doesn't → a latent
  under-copy (would share a binder that should be re-resolved). Given a5f7cfafb fixed a regression by
  NARROWING to own-binders, (a) is likely — but v-patterns should confirm a nested-fused-match-in-arm
  witness still lowers correctly before just deleting the doc clause.
Verify-the-behavior-first (don't just delete the doc): construct a fused match whose cloned arm contains
its OWN nested match binder, confirm it copies correctly, THEN reconcile doc↔code.

Owner: **v-patterns** (`rcdzc/src/lower.rs`, match-fusion arm-clone; blame a5f7cfafb). Doc claims a
"within the cloned subtree" copy case the code dropped when it narrowed to `fused_scrut == Some(scrutinee)`.
Confirm the narrow check is complete (nested-fused-in-arm witness) → then either delete the stale doc
clause (3 sites: ~5208/5212/5230) or restore the subtree case if a real under-copy exists.
