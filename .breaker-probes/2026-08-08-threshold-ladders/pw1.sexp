(case "pw1 a TRIPLING state crosses fixed thresholds — three compares catch the crossing at input-dependent depth"
  (input  (do
            (effect E (op over (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E (if (> n 0) n 1)
                ((over (th) s (resume (if (> s th) 1 0) (* s 3))))
                (+ (* 100 (E.over 4)) (+ (* 10 (E.over 40)) (E.over 400)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 100 Int64))
  (call   main (: 20 Int64)) (output (: 110 Int64))
  (call   main (: 150 Int64)) (output (: 111 Int64)))
