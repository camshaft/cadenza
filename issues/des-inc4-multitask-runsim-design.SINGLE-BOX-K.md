# inc-4 multi-task run-sim — design (single-payload Box for k, per v-effects note 9470)

**Blocked on:** MR `b3ffca83e` (v-effects + v-inference: `fold_ctor_match` + SumPayload substitution)
landing on trunk. Verify by re-running the inc-4 repro → 5e9 before building.

## Critical constraint (v-effects note 9470)
The landed slice folds **single-payload single-binder** ctor patterns — e.g. `(Box.Box th)`. A
**multi-payload** match pattern (2+ binders like `(Entry waketime k)`, or a nested `(tuple ..)` payload)
where the extracted continuation is then applied will **decline** until `fold_ctor_match` is extended to
the multi-payload path. **So: box each continuation SINGLY.** The continuation `k` must be extracted from
a `(Box.Box k)` single-binder pattern, NOT destructured inline from a `(tuple waketime k ...)`.

### Consequence for the pqueue
The PRE-STEP3 skeleton stored `(Tuple Instant Int64 PQ)` and popped via
`(PQ.PQCons (tuple ht hv rest))` — a MULTI-binder tuple pattern. That is fine for the *waketime* and
*rest* (they are plain values, not applied continuations), BUT the continuation payload must NOT be
extracted-and-applied from that same multi-binder tuple. Design so the continuation rides in its own
single-payload `Box`, and the apply site matches ONLY `(Box.Box k)`:

```
(type KBox (KBox (-> Unit Instant)))                 ; single-payload box holding the resume-thunk
(type PQ PQNil (PQCons (Tuple Instant KBox PQ)))     ; entry = (waketime, boxed-k, rest)
(def (apply-kbox (: b KBox)) (match b ((KBox.KBox k) (k unit))))   ; SINGLE-binder extract + apply
```

pop-min returns `(waketime, KBox, rest-of-pq)`; the scheduler sets clock := waketime, then calls
`apply-kbox` on the popped box — the apply flows through the single-binder `(KBox.KBox k)` pattern the
fold covers. The `(tuple ht hbox rest)` tuple match only *reads* hbox as a value and passes it to
`apply-kbox`; it does not apply the continuation inline. If even reading a Box out of a tuple then
applying it declines (i.e. the fold needs the box match to be the DIRECT arm, not one-hop through a
tuple), fall back to: pop-min returns the KBox, and a separate top-level `apply-kbox` is called on it
(the landed repro's exact shape — Box built, unbox-apply match-extracts + applies).

## The §4.2 2-worker interleave (value grade)
Two workers spawned, sleeping to 1s and 3s; main sleeps to 5s. Woken order (deterministic FF clock):
B@1s, A@3s, main@5s. Value-grade the emit trace as "B,A,main" (string append in woken order) OR, if
string-building is heavy, grade the final observed clock (5e9) + a woken-order accumulator (e.g. an
Int64 encoding 1,3,5 order). Keep the first inc-4 corpus case minimal: TWO tasks + main, graded on the
woken-order witness, then expand to 3-task tie / deep-8 in follow-ups.

## Build order when b3ffca83e lands
1. sync clean; rebuild runtime (`cargo xtask build`) if corpus shows stale-runtime FAILs.
2. Re-run the inc-4 repro → confirm 5e9 on trunk.
3. Build the single-box run-sim over the schedstate skeleton; gate all 3 backends.
4. If pop-min's tuple-then-apply declines → minimal repro to v-effects+v-inference (multi-payload
   fold_ctor_match extend), fall back to top-level apply-kbox meanwhile.
5. Add the §4.2 2-worker "B,A,main" case; promote; update 3 baselines (surgical single-line inserts,
   NOT a full sort — baselines are tool-collated).
6. MR to pr-sync; keep single-task corpus locked.
