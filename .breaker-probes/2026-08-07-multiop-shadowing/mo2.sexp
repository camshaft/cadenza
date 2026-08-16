(case "mo2 THREE-deep same-effect shadowing — each depth's draws thread its own state, interleaved before/inside/after"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (+ (St.next)
                   (+ (handle St 100
                        ((next () s (resume s (+ s 20))))
                        (+ (St.next)
                           (+ (handle St 7
                                ((next () s (resume s (* s 3))))
                                (+ (St.next) (St.next)))
                              (St.next))))
                      (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 259 Int64))
  (call   main (: 0 Int64)) (output (: 249 Int64)))
