(case "ph2 a perform BETWEEN two slices of one rope (the view construction brackets the effect)"
  (input  (do
            (effect St (op mark (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((mark (u) s (resume s (+ s 1))))
                (do
                  (def b (Bytes.of (list 10 20 30 40)))
                  (def s1 (Option.expect (Bytes.slice b 0 2) "lo"))
                  (def m (St.mark))
                  (def s2 (Option.expect (Bytes.slice b 2 2) "hi"))
                  (+ (* 100 m)
                     (+ (* 10 (match (Bytes.at s1 0) ((Some v) (Int64.of v)) ((None _u) -1)))
                        (match (Bytes.at s2 0) ((Some v) (Int64.of v)) ((None _u) -1)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 630 Int64)))
