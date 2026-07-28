# mlrepro: a TWO-REAL-CTOR-ARM cross-variant boxed match traps compiled (wasm unreachable)

Found by: v-compiler-ml (b148 wildcard bounce, isolated). Status: OPEN — separate from the _-wildcard slice.

SYMPTOM: `(do (type PQ (P Int64 Int64) (Q Int64)) (def (go n) (match (if (< n 1) (Q 0) (P 3 4)) ((P _ y) y) ((Q z) z))) (def (main) (go 5)) (export main))` TRAPS `wasm unreachable` on the COMPILED path (b148 gate). The distinguishing shape vs all GREEN witnesses: TWO REAL ctor-pattern arms (`((P _ y) y)` AND `((Q z) z)`) over a boxed mixed decl, with a CROSS-VARIANT runtime scrutinee `(if (< n 1)(Q 0)(P 3 4))` — NO `_` catch-all arm. Every green multifield/wildcard witness has a `(_ N)` catch-all as the final arm; this has two ctor arms and no catch-all.

NARROWED: NOT the _-wildcard binder (direct-Core CMatchSum(CCtor(0,[3,4]),0,[-1,101],CVar 101)→4 passes compiled; the wildcard slice re-sent green-shaped as 1913a15cb). NOT the multifield deconstruct (green). The SUSPECT is the nested-CMatchSum chain for TWO ctor arms (P-arm then Q-arm, no CIf-else terminal) over a boxed scrutinee — possibly the 2nd arm's store-tag re-test or the missing-catch-all else-path emit. Likely same width-disjoint-slot / emit family as the earlier sum-payload traps.

REPRO the isolation: change the green ss-multifield-payload-ctor-runtime-boxed by replacing its `(_ 0)` catch-all with a real 2nd ctor arm `((Q z) z)` + a `(Q Int64)` sibling + cross-variant if → traps. Keep the `(_ N)` catch-all → green.

NEXT: needs the boundary-Int64 probe chain (which stage: does lower produce the nested-CMatchSum for 2 ctor arms? does eval bind-payload on the 2nd arm work? is it the emit of a CMatchSum whose else is another CMatchSum, not a CNum?). ROUTE candidates: v-compiler-ml (if lower/eval logic) or v-wasm-opt (if emit). Deferring until the wildcard + Bool MRs land (don't stack on the same files).
