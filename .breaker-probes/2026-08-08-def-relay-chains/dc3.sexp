(case "dc3 the MIDDLE relay hop is chosen by a draw — the branch decides between a two-draw and a one-draw callee, a tail draw pins the total"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (g2) (+ (* 10 (E.next)) (E.next)))
            (def (h1) (E.next))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (* 100 (if (> (E.next) 0) (g2) (h1)))
                   (- (E.next) n))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 5603 Int64))
  (call   main (: 0 Int64)) (output (: 102 Int64))
  (call   main (: -3 Int64)) (output (: -198 Int64)))
