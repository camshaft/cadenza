(case "il2 the interleave WIDTH is data-driven — parity picks one-or-two O draws per I tick, a helper sums each O burst"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect I (op tick (-> Int64)))
            (def (burst (: k Int64))
              (if (= k 2) (+ (O.next) (O.next)) (O.next)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle I 100
                  ((tick () t (resume t (+ t 2))))
                  (let ((k (if (= (% n 2) 0) 2 1)))
                    (+ (burst k)
                       (+ (* 10 (I.tick))
                          (+ (* 100 (burst k))
                             (* 1000 (I.tick)))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 103403 Int64))
  (call   main (: 2 Int64)) (output (: 103905 Int64))
  (call   main (: -1 Int64)) (output (: 102999 Int64)))
