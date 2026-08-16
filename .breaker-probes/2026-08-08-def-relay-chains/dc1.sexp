(case "dc1 a THREE-hop def relay under ONE handler — each def draws then calls the next, weights pin the depth order"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (h3) (E.next))
            (def (g2) (+ (* 10 (E.next)) (h3)))
            (def (f1) (+ (* 100 (E.next)) (g2)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (f1)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 345 Int64))
  (call   main (: 0 Int64)) (output (: 12 Int64))
  (call   main (: -2 Int64)) (output (: -210 Int64)))
