(case "dn3 an ALTERNATING A/B/A/B tower where both effects share the op NAME next — each perform homes to its effect's innermost handler"
  (input  (do
            (effect A (op next (-> Int64)))
            (effect B (op next (-> Int64)))
            (def (main (: n Int64))
              (handle A n
                ((next () s (resume s (+ s 1))))
                (handle B 10
                  ((next () s (resume s (+ s 2))))
                  (handle A 100
                    ((next () s (resume s (+ s 3))))
                    (handle B 1000
                      ((next () s (resume s (+ s 4))))
                      (+ (A.next) (+ (B.next) (+ (A.next) (B.next)))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 2207 Int64)))
