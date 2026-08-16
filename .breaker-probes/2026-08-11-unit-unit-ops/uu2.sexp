(case "uu2 a UNIT handler state — the stateless idiom threads unit through a recursive doubling walk"
  (input  (do
            (effect L (op double (-> Int64 Int64)))
            (def (wk (: k Int64) (: acc Int64))
              (if (< k 1) acc (wk (- k 1) (+ acc (L.double k)))))
            (def (main (: n Int64))
              (handle L unit
                ((double (v) u (resume (* 2 v) u)))
                (wk n 0)))
            (export main)))
  (call   main (: 4 Int64)) (output (: 20 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))
