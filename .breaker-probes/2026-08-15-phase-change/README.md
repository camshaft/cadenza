# phs — phase-change heater: a NEW decline shape (2026-08-15, tick 1528)

Heater with a latent-heat plateau: tick either raises temp or, exactly at the
plateau with latent unpaid, pays 2 latent. The plateau guard reads BOTH tuple
fields: outer `(= temp K)`, inner `(< latent 6)`, and the two branches update
DIFFERENT fields (latent-branch writes latent, rise-branch writes temp).

| probe | variation | verdict |
|-------|-----------|---------|
| phs1 | original (negative domain, 2 ops) | DECLINE ×3 |
| phs2 | nested-if flattened to (and …) | DECLINE |
| phs3 | all-positive domain | DECLINE |
| phs4 | single-op (stat dropped) | DECLINE |
| phs5 | inner guard made CONSTANT (< 0 6) | **PASS** |

Isolated: a 2-level guard where the INNER condition reads a DIFFERENT state
field than the outer, with each branch updating its own field, declines at
6 straight-line dispatches — even single-op (so NOT lstM's mixed-op class,
NOT medK's callee-let class, and the and-flattening doesn't save it).
Constant inner guard compiles. New face for the decline-frontier family;
phs5 (the passing face) is corpus-eligible.
