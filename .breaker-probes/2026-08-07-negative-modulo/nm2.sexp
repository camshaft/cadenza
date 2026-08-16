(case "nm2 NEGATIVE division TRUNCATES toward zero uniformly — bare and through a handler arm"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (+ (* 1000 (/ n 3))
                 (handle St n
                   ((next () s (resume (/ s 3) (+ s 2))))
                   (+ (St.next) (* 10 (St.next))))))
            (export main)))
  (call   main (: -7 Int64)) (output (: -2012 Int64))
  (call   main (: 7 Int64)) (output (: 2032 Int64)))
