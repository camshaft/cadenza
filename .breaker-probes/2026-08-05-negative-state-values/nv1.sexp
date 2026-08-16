(case "nv1 NEGATIVE handler state values through advance/observe (sign preservation in state slot)"
  (input  (do
            (effect St (op sub (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St n
                ((sub (v) s (resume s (- s v))))
                (+ (* 100 (St.sub 10)) (+ (* 10 (St.sub 10)) (St.sub 10)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 435 Int64)))
