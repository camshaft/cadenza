(case "mo3 bisect: module-exported NON-recursive performer (single perform)"
  (input  (do
            (effect Ctr (op next (-> Unit Int64)))
            (module m
              (def (once (: k Int64)) (+ k (Ctr.next unit)))
              (export once))
            (def (main (: n Int64))
              (handle Ctr n
                ((next (u) s (resume s (+ s 1))))
                ((. m once) 100)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 105 Int64)))
