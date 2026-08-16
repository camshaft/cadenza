(case "a DES-shaped scheduler loop drains time-keyed events applying each to threaded state"
  (doc    "The event-loop composition: a time-keyed CHAMP queue drained by min-extraction, each
           event's payload APPLIED to a threaded state with an order-sensitive step (st·2 + v — the
           doubling makes every position distinct): events at times {k,20,30} apply in TIME order
           regardless of insertion (k=10 → payloads 1,2,3 → 11; k=40 → 2,3,1 → 15). Composes the
           min-extract drain (DM/EC pin the extraction order alone) with a STATE FOLD across the
           drain — the discrete-event-simulation main loop in miniature, where a mis-ordered
           extraction changes not just a digit but the whole accumulated state trajectory.")
  (input  (do
            (def (run (: q (Map Int64 Int64)) (: st Int64) (: fuel Int64))
              (if (= fuel 0)
                st
                (if (= (Map.len q) 0)
                  st
                  (match (List.at (Map.to-list q) 0)
                    ((Some e)
                      (run (Map.remove q (. e 0))
                           (+ (* st 2) (. e 1))
                           (- fuel 1)))
                    ((None u) st)))))
            (def (main (: k Int64))
              (run (Map.insert (Map.insert (Map.insert Map.empty 30 3) k 1) 20 2) 0 10))
            (export main)))
  (call   main (: 10 Int64)) (output (: 11 Int64))
  (call   main (: 40 Int64)) (output (: 15 Int64)))
