(case "nm1 NEGATIVE remainder is TRUNCATED (sign of the dividend) uniformly — bare and through a handler arm"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (+ (* 1000 (% n 7))
                 (handle St n
                   ((next () s (resume (% s 7) (- s 5))))
                   (+ (St.next) (* 10 (St.next))))))
            (export main)))
  (call   main (: -10 Int64)) (output (: -3013 Int64))
  (call   main (: 10 Int64)) (output (: 3053 Int64))
  (call   main (: -7 Int64)) (output (: -50 Int64)))
