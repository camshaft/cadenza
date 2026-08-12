# 2026-08-12 width-partition siblings — post-#21-fix probes (base 8fad2b1ee, fix aaee597d5 in)

Siblings of finding #21 (i64 checked-arith scratch aliased with i32 Option-handle slot in a
handler arm with two lookup-matches + computed perform arg). Fix = width-partition; probing
whether OTHER scratch widths / arities re-trip it.

- `wp1.sexp` — Float64 computed perform arg: **uniform decline ×3** (todo wasm=rust=rust-async), not a bug.
- `wp2.sexp` — checked-shift `(<< n 2)` computed arg: PASS ×3. Pin candidate.
- `wp3.sexp` — three-param op, two computed args: PASS ×3. Pin candidate.

## #21 close-out verification (fresh detached origin 8fad2b1ee, rebuilt cdz + store)
mml1 + minT + minK all PASS ×3 (wasm was the invalid artifact pre-fix).
corpus-bugfix pinned the single minT witness (landed, content-verified on origin 14-effects);
my fold batch (238) = mml1 + minK + wp2 + wp3, distinct titles, no baseline collision.

## Tick 1320 additions (base 8fad2b1ee)
- `rmp1.sexp` — record state {m: Map, cnt}: computed keys, arm observes BOTH fields
  (10*lookup + advanced counter). PASS ×3. Pin candidate for batch-238.
  (First draft had an unread cnt — weak-pin rule caught it; second draft had an
  arithmetic slip in the expectation — the gate caught it, model+compiler agree 3182.)
- `lp1-declines.sexp` / `bp1-declines.sexp` — List.at / Bytes.at as the Option producers
  with a computed perform arg: UNIFORM todo ×3 (fold declines; not a bug, decline witnesses).
  NOTE ids lp1/bp1 are TAKEN in 14c — rename before any future pin.

## Tick 1321 additions (base 8fad2b1ee)
- `ab21.sexp` — the #21 shape on the ABORT path (arm answers without resume, value built
  from two lookup-matches over a computed-key insert; seed n*3 differentiates). PASS ×3.
- `ch21.sexp` — CHAINED computed keys: second perform's key computed from the first's
  answer, both dispatches through the two-lookup-match arm. PASS ×3.
Batch-238 set: mml1, minK, wp2, wp3, rmp1, ab21, ch21 (7 pins).

## Tick 1322 addition (base 47d887469)
- `xh1.sexp` — CROSS-HANDLER: inner arm performs the outer op with a computed key
  (+ s 1) built from the inner state; outer arm is the two-lookup-match Map shape.
  The #21 partition across a handler boundary, state feedback s+t between dispatches.
  PASS ×3. Batch-238 set now 8: mml1, minK, wp2, wp3, rmp1, ab21, ch21, xh1.
