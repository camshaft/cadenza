(case "abl1 a FOREIGN LEVY BEFORE AN ABORT — the inner arm levies the outer handler then answers WITHOUT resuming so the body's pending addition is abandoned, the levy's state write survives the abandonment and surfaces in the outer audit, and dropping the doomed frame's levy (it precedes an abort so a lowering might skip the whole arm prefix) shifts the audit digit while the abort value holds"
  (input  (do
            (effect T (op levy (-> Int64)) (op audit (-> Int64)))
            (effect E (op bail (-> Int64)))
            (def (main (: n Int64))
              (handle T (% n 3)
                ((levy () t (resume t (+ t 5)))
                 (audit () t (resume t t)))
                (+ (* 100 (handle E (: 1 Int64)
                            ((bail () s
                              (do (T.levy) (+ s 900))))
                            (+ (E.bail) 7)))
                   (T.audit))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 90106 Int64))
  (call   main (: 0 Int64)) (output (: 90105 Int64)))
