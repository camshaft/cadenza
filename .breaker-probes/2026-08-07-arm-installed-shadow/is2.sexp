(case "is2 the ARM's resume-value is a nested SAME-effect handle — a fresh shadow instantiated per dispatch from the live state"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume (handle St (* s 10)
                                      ((next () t (resume t (+ t 1))))
                                      (+ (St.next) (St.next)))
                                    (+ s 1))))
                (+ (St.next) (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 222 Int64))
  (call   main (: 2 Int64)) (output (: 102 Int64)))
