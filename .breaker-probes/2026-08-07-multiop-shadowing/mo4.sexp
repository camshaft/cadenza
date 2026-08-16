(case "mo4 a SAME-effect draw in the inner handle's SEED homes to the OUTER handler — the shadow starts at the body, not the seed"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (+ (handle St (* (St.next) 10)
                     ((next () s (resume s (+ s 100))))
                     (+ (St.next) (St.next)))
                   (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 206 Int64))
  (call   main (: 2 Int64)) (output (: 143 Int64)))
