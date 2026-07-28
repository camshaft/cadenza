# mlrepro: a TWO-REAL-CTOR-ARM cross-variant boxed match traps compiled (wasm unreachable)

Found by: v-compiler-ml (b148 wildcard bounce, isolated). Status: OPEN — separate from the _-wildcard slice.

SYMPTOM: `(do (type PQ (P Int64 Int64) (Q Int64)) (def (go n) (match (if (< n 1) (Q 0) (P 3 4)) ((P _ y) y) ((Q z) z))) (def (main) (go 5)) (export main))` TRAPS `wasm unreachable` on the COMPILED path (b148 gate). The distinguishing shape vs all GREEN witnesses: TWO REAL ctor-pattern arms (`((P _ y) y)` AND `((Q z) z)`) over a boxed mixed decl, with a CROSS-VARIANT runtime scrutinee `(if (< n 1)(Q 0)(P 3 4))` — NO `_` catch-all arm. Every green multifield/wildcard witness has a `(_ N)` catch-all as the final arm; this has two ctor arms and no catch-all.

NARROWED: NOT the _-wildcard binder (direct-Core CMatchSum(CCtor(0,[3,4]),0,[-1,101],CVar 101)→4 passes compiled; the wildcard slice re-sent green-shaped as 1913a15cb). NOT the multifield deconstruct (green). The SUSPECT is the nested-CMatchSum chain for TWO ctor arms (P-arm then Q-arm, no CIf-else terminal) over a boxed scrutinee — possibly the 2nd arm's store-tag re-test or the missing-catch-all else-path emit. Likely same width-disjoint-slot / emit family as the earlier sum-payload traps.

REPRO the isolation: change the green ss-multifield-payload-ctor-runtime-boxed by replacing its `(_ 0)` catch-all with a real 2nd ctor arm `((Q z) z)` + a `(Q Int64)` sibling + cross-variant if → traps. Keep the `(_ N)` catch-all → green.

NEXT: needs the boundary-Int64 probe chain (which stage: does lower produce the nested-CMatchSum for 2 ctor arms? does eval bind-payload on the 2nd arm work? is it the emit of a CMatchSum whose else is another CMatchSum, not a CNum?). ROUTE candidates: v-compiler-ml (if lower/eval logic) or v-wasm-opt (if emit). Deferring until the wildcard + Bool MRs land (don't stack on the same files).

---
UPDATE 2026-07-28 (post-wildcard-land): NARROWED + CAVEAT. Direct-Core probe (compiles locally): a nested
CMatchSum whose inner rest is a poison CVar(0) (the no-catch-all terminal) EVALS the taken arm cleanly compiled
(CMatchSum(CCtor(0,[3,4]),0,[-1,101],CVar101, CMatchSum(scrut,1,[102],CVar102,CVar0)) → 4). So the EVAL logic for
a two-ctor-arm nested-CMatchSum-with-poison-rest is SOUND compiled — the trap (if real) is NOT eval; it's upstream
(reader/infer Core shape at self-host scale) or emit. ⚠ CAVEAT: the original b148 witness that produced this finding
ALSO had the _-wildcard confound (((P _ y) y)((Q z) z)) — and the wildcard slice has since landed GREEN (1913a15cb)
with a proper proven-shape witness. So this finding may be CONFLATED with the (now-fixed) wildcard-witness bug.
NEXT (decline-first per concierge discipline — cannot compiled-verify e2e locally, CDZ0999): build a CLEAN isolation
witness with NO wildcard — `(match (if (< n 1)(Q 0)(P 3 4)) ((P x y)(+ x y)) ((Q z) z))` two REAL ctor arms, no `_`,
no catch-all — and let pr-sync's compiled gate be the oracle. If it RUNS → this finding was a wildcard-witness
artifact, close it. If it TRAPS → real, localize reader/infer/emit. Ship the isolation witness as a DECLINE-first
pin (or run it via pr-sync only) — do NOT ship a RUN-asserting trap-risk test (the 3-bounce lesson).

---
RESOLUTION 2026-07-28: LIKELY ARTIFACT — downgrading. Three facts converge: (1) direct-Core eval of a two-ctor-arm
nested-CMatchSum-poison-rest is SOUND compiled (→4); (2) a wildcard-LESS ctor-pattern match with no catch-all
ALREADY declines cleanly + is pinned GREEN on trunk (ss-ctor-pattern-match-no-wildcard-declines-cleanly); (3) the
original b148 trap witness had the _-wildcard confound (((P _ y) y)…), and the wildcard slice has since LANDED
GREEN (1913a15cb) with a proper proven-shape witness. So the "two-ctor-arm traps" was almost certainly the
wildcard-witness bug (now fixed), NOT a separate real trap. A clean-isolation e2e (((P x y)(+ x y))((Q z) z), no
wildcard, no catch-all, cross-variant) via pr-sync's compiled gate would CONFIRM, but this is LOW priority — the
sound behavior (decline on no-catch-all; run on covered arms) is already pinned. Not building a dedicated witness
(would be a decline-first pin duplicating existing coverage). Re-open only if a real trap resurfaces in a gate.
