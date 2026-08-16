(case "sq2 an ABORT in the first handle leaves the SECOND handle's dispatch untouched"
  (input  (do
            (effect Bail (op stop (-> Int64 Int64)))
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (+ (* 10 (handle Bail 0
                         ((stop (v) s (* v 2)))
                         (+ 999 (Bail.stop n))))
                 (handle St 7
                   ((next (u) s (resume s (+ s 1))))
                   (+ (St.next) (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 115 Int64)))
