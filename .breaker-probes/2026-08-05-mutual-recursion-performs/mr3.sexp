(case "mr3 mutual recursion against TWO different handlers (each side its own effect, nested frames)"
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (effect B (op b (-> Unit Int64)))
            (def (pa (: k Int64))
              (if (= k 0) 0 (+ (* 10 (A.a)) (pb (- k 1)))))
            (def (pb (: k Int64))
              (if (= k 0) 0 (+ (B.b) (pa (- k 1)))))
            (def (main (: n Int64))
              (handle A n
                ((a (u) s (resume s (+ s 1))))
                (handle B 100
                  ((b (u) t (resume t (+ t 10))))
                  (pa 4))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 320 Int64)))
