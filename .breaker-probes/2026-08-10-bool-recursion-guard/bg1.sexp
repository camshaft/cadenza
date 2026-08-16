(case "bg1 a BOOL op result is the recursion CONDITION itself — the walk continues while draws stay true"
  (input  (do
            (effect T (op more (-> Bool)) (op tick (-> Int64)))
            (def (walk (: acc Int64))
              (if (T.more)
                  (walk (+ acc (T.tick)))
                  acc))
            (def (main (: n Int64))
              (handle T n
                ((more () s (resume (< s 4) s))
                 (tick () s (resume s (+ s 1))))
                (+ (* 10 (walk 0)) (T.tick))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 64 Int64))
  (call   main (: 4 Int64)) (output (: 4 Int64)))
