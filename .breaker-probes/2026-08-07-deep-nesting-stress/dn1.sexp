(case "dn1 a FIVE-deep same-effect shadow tower — one draw per level plus a doubled draw at the innermost"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (+ (St.next)
                   (handle St 10
                     ((next () s (resume s (+ s 2))))
                     (+ (St.next)
                        (handle St 100
                          ((next () s (resume s (+ s 3))))
                          (+ (St.next)
                             (handle St 1000
                               ((next () s (resume s (+ s 4))))
                               (+ (St.next)
                                  (handle St 10000
                                    ((next () s (resume s (+ s 5))))
                                    (+ (St.next) (St.next))))))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 21120 Int64))
  (call   main (: 0 Int64)) (output (: 21115 Int64)))
