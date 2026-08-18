(case "pyd2 the PRE-SUSPEND BINDING SURVIVES BOTH REPLAYS — a hundredfold toll is let-bound before either resume so the saved slot must ride through the discarded first replay AND the surviving second one before its single consumption, a slot refreshed per replay or dropped by the discard would misprice the hundreds, and the low digits carry the surviving replay's identity"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (let ((t (* 100 (+ s 1))))
                    (do (resume s (+ s 1))
                        (+ (resume (+ s 10) (+ s 2)) t)))))
                (E.tick)))
            (export main)))
  (call   main (: 10 Int64)) (output (: 211 Int64))
  (call   main (: 0 Int64)) (output (: 110 Int64)))
