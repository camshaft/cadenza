(case "a performing closure dispatched through a TUPLE projection threads the handler state"
  (doc    "The container-dispatch face of the performing closure (:566 pins the let-bound closure
           called twice): here the closure lives in a TUPLE element and BOTH calls reach it through
           the projection `(. t 0)` — each projected application is a fresh perform against the
           CURRENT state (reads n then n+1 → 10n + n+1 = 34 at n=3). The homing walk must find the
           perform through the container round-trip, and the state must thread across two separate
           projections of the SAME stored closure (a per-projection env copy that re-seeded, or a
           first-discharge replay, gives 33). Tuple is the fixed-shape container; the CHAMP-map
           dispatch twin still declines (banked TODO) — this pins the working rep.")
  (input  (do
            (effect Ctr (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Ctr n
                ((next (u) s (resume s (+ s 1))))
                (let ((t (tuple (fn ((: u Unit)) (Ctr.next unit)) 9)))
                  (+ (* 10 ((. t 0) unit)) ((. t 0) unit)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 34 Int64)))
