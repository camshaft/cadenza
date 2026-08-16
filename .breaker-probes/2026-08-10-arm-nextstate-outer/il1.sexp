(case "il1 strict O-I-O-I body interleave — two independent threads advance in lockstep, positional weights pin the dispatch order"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect I (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle I 100
                  ((tick () t (resume t (+ t 2))))
                  (+ (O.next)
                     (+ (* 10 (I.tick))
                        (+ (* 100 (O.next))
                           (* 1000 (I.tick))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 103403 Int64))
  (call   main (: 0 Int64)) (output (: 103100 Int64)))
