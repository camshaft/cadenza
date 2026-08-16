(case "aa5 DIVISION in the resume value with a subtracting stride — quotients shrink as the state walks down"
  (input  (do
            (effect E (op g (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((g () s (resume (/ s 3) (- s 4))))
                (+ (E.g) (+ (* 10 (E.g)) (* 100 (E.g))))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 790 Int64))
  (call   main (: 9 Int64)) (output (: 13 Int64)))
