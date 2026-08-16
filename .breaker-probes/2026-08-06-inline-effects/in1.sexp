(case "in1 a performing helper called from TWO sites — each call site is its own dispatch"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (step (: k Int64)) (+ k (St.next)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (+ (* 100 (step 1)) (step 2))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 608 Int64)))
