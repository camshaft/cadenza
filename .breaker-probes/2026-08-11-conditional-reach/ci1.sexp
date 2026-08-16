(case "ci1 a CONDITIONALLY-recursive helper in body position — one branch reaches the recursive performer, the other is a constant, both taken"
  (input  (do
            (effect S (op tick (-> Int64)))
            (def (inner (: k Int64) (: acc Int64))
              (if (< k 1) acc (inner (- k 1) (+ acc (S.tick)))))
            (def (maybe (: go Int64))
              (if (= go 1) (inner 2 0) 55))
            (def (main (: n Int64))
              (handle S n
                ((tick () s (resume s (+ s 1))))
                (+ (maybe 1) (* 1000 (maybe 0)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 55003 Int64))
  (call   main (: 0 Int64)) (output (: 55001 Int64)))
