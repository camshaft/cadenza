(case "hc4 a performing helper COMPOSED with itself — probe(probe(draw)), three dispatches through two call frames"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (probe (: k Int64)) (+ (St.next) (* k 10)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (probe (probe (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 567 Int64))
  (call   main (: 0 Int64)) (output (: 12 Int64)))
