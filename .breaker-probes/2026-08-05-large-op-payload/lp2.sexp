(case "lp2 the large payload MUTATES per hop through a 3-perform pipeline (arm appends each time)"
  (input  (do
            (effect St (op hop (-> (List Int64) (List Int64))))
            (def (main (: n Int64))
              (handle St 0
                ((hop (xs) s (resume (List.push xs (List.len xs)) s)))
                (do
                  (def a (St.hop (list n)))
                  (def b (St.hop a))
                  (def c (St.hop b))
                  (+ (* 100 (List.len c))
                     (match (List.at c 3) ((Option.Some v) v) ((Option.None) -1)))))
            )
            (export main)))
  (call   main (: 7 Int64)) (output (: 403 Int64)))
