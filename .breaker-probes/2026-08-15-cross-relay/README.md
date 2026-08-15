# rly — cross-effect relay: binder-over-PERFORM declines (2026-08-15, tick 1542)

B's handler arm performs the outer A (same-shape twins, arm-perform routing
under schema-hash-only identity).

| probe | shape | verdict |
|-------|-------|---------|
| rly1 | (match (A.bump …) (got …)) DUAL-use of got | DECLINE ×3 |
| rlyC | same binder, SINGLE-use | DECLINE |
| rlyB | direct (resume (A.bump …) s), no binder | **PASS ×3** |

The decline is the MATCH BINDER OVER A PERFORM-expression scrutinee — the
binder itself, not the dual-use (rlyC kills that hypothesis) and not the
arm-perform (rlyB passes and pins cross-effect identity through an arm-
perform end-to-end). Same family as kgt0 (binder-over-IF): the binder-
scrutinee fence covers conditionals AND performs; call/arithmetic compounds
remain fine (rps2/lru1/brd1/tie1 all landed).

rlyB is corpus-eligible: same-shape twins + arm-perform relay, answers prove
A advanced by the relay while B's state held. Flip-watch: rly1 with kgt0.
