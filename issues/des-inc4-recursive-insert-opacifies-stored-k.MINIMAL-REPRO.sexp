; ═══════════════════════════════════════════════════════════════════════════════════════════════════
; DES inc-4 ⇄ v-effects/v-inference — MINIMAL REPRO: a continuation routed through a RECURSIVE insert
; (pqueue sorted-insert) then popped+applied DECLINES; a DIRECTLY-constructed entry folds. The last reach.
; from v-discrete-event-sim. trunk e6fd73bfa.
; ═══════════════════════════════════════════════════════════════════════════════════════════════════
; CONTEXT: landed reaches fold a continuation stored in a DIRECTLY-CONSTRUCTED pqueue entry and popped via
; a 2-level match — `(sched-step (PQCons (tuple wake (KBox k) PQNil)))` → 5e9 (VERIFIED passing on trunk,
; promoted to corpus). But a real time-ordered pqueue inserts via a RECURSIVE sorted-insert `pins` to keep
; entries ascending by waketime. Routing the boxed continuation through `pins` (recursion) then popping it
; DECLINES — the recursive insert opacifies the stored continuation to the deferred-resume fold.
;
; BISECTION (identical except how the entry reaches sched-step):
;   direct `(sched-step (PQCons (tuple wake (KBox k) PQNil)))` .................... PASS → 5e9
;   via recursive `(sched-step (pins PQNil wake (KBox k)))` ....................... DECLINES  ← THIS FILE
; `pins` is an ordinary recursive sorted-insert (match q; if before? cons else recurse). The continuation
; k = `(fn (_u) (resume unit wake))` closes over wake exactly as in the passing case; the ONLY change is
; that it flows through pins's recursion before the pop.
;
; WHY inc-4 needs it: a priority queue maintains time-order via recursive sorted-insert; the scheduler
; files each task's (waketime, k) with `pins` then pops the min. Without seeing the continuation through
; `pins`, the multi-task run-sim can't fold. EXPECTED once the deferred-resume fold sees through a
; recursive insert (the stored KBox survives the recursion): (: 5000000000 Int64).

(do
  (type Instant (Instant UInt64))
  (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
  (def (before? (: a Instant) (: b Instant)) (< (inst-ns a) (inst-ns b)))
  (type KBox (KBox (-> Unit Unit)))
  (type PQ PQNil (PQCons (Tuple Instant KBox PQ)))
  ; recursive sorted-insert: keep entries ascending by waketime
  (def (pins (: q PQ) (: t Instant) (: kb KBox))
    (match q
      ((PQ.PQNil _) (PQ.PQCons (tuple t kb (PQ.PQNil ()))))
      ((PQ.PQCons (tuple ht hk r))
        (if (before? t ht)
            (PQ.PQCons (tuple t kb (PQ.PQCons (tuple ht hk r))))
            (PQ.PQCons (tuple ht hk (pins r t kb)))))))
  ; pop-min + apply the boxed continuation (2-level match — folds when the entry is DIRECT)
  (def (sched-step (: q PQ))
    (match q
      ((PQ.PQNil _) unit)
      ((PQ.PQCons (tuple wake kb rest)) (match kb ((KBox.KBox k) (k unit))))))
  (effect Sim
    (op sleep (-> Instant Unit))
    (op now   (-> Unit Instant)))
  (def (main)
    (handle Sim (Instant.Instant 0)
      ( (now   (u) s (resume s s))
        (sleep (wake) s
          (sched-step (pins (PQ.PQNil ()) wake (KBox.KBox (fn (_u) (resume unit wake)))))) )   ; via recursive pins
      (do (Sim.sleep (Instant.Instant 5000000000))
          (inst-ns (Sim.now)))))
  (export main))
; EXPECTED once the fold sees through a recursive insert: (: 5000000000 Int64). TODAY: declines cleanly.
