(case "sf1 an arm re-performs its OWN effect to a SAME-EFFECT outer handler — the true self-shadow forward"
  (input  (do
            (effect Ctr (op bump (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Ctr 100
                ((bump (v) t (resume (+ v t) (+ t 1))))
                (handle Ctr 0
                  ((bump (v) s (resume (Ctr.bump (* v 10)) s)))
                  (Ctr.bump n))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 150 Int64)))
