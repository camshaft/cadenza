(case "aa2 the shared helper is RECURSIVE and the arms call it on their heap state"
  (input  (do
            (effect St (op push (-> Int64 Int64)) (op sum (-> Unit Int64)))
            (def (suml (: xs (List Int64)))
              (match xs ((list) 0) ((list h .. t) (+ h (suml t)))))
            (def (main (: a Int64))
              (handle St (list)
                ((push (v) s (resume (suml s) (List.push s v)))
                 (sum (u) s (resume (suml s) s)))
                (+ (* 100 (St.push a)) (+ (* 10 (St.push (+ a 1))) (St.sum)))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 85 Int64)))
