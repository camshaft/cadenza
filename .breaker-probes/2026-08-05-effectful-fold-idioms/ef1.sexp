(case "ef1 the running-MAXIMUM idiom: a fold whose combiner performs, tracking extrema through effect state"
  (input  (do
            (effect St (op see (-> Int64 Int64)))
            (def (walk (: xs (List Int64)) (: acc Int64))
              (match xs
                ((list) acc)
                ((list h .. t) (walk t (+ acc (St.see h))))))
            (def (main (: n Int64))
              (handle St 0
                ((see (v) s (resume (if (> v s) 1 0) (if (> v s) v s))))
                (+ (* 10 (walk (list 3 9 2 9 12) 0)) 0)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 30 Int64)))
