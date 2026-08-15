# trn — turnstile FSM: F24 body-size at 7 dispatches, 6 passes (2026-08-15, tick 1508)

Coin/push turnstile over a 4-tuple (unlocked, waste, bounce, passed): coin
unlocks-or-wastes, push passes-and-relocks-or-bounces (both 2-branch, CHEAP
branches — single +1 counters, no compound recomputes). Seed decides the
starting lock state; the very first row diverges; n=0 packs negative.

- trn1-explodes: 7 dispatches → INVALID ×3, 11,972,500-byte emit, BODY-SIZE
  kind.
- trn6: 6 dispatches → PASS. trn5: 5 dispatches → PASS ×1 (gate green).

Envelope datapoint: CHEAP 2-branch arms × 4-TUPLE broke at 7 (bkf1's cheap
2-branch × 2-tuple was green at 8). So tuple WIDTH multiplies the per-dispatch
cost even with cheap branches — consistent with per-dispatch duplication of
the whole state-rebuild. Sixth F24 hit. trn6 (the passing 6-dispatch face) is
corpus-eligible. trn1 held on the F24 watch.
