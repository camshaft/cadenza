(case "av4 the abort value dispatches to the SAME handler's resumptive SIBLING op — re-entrant own-frame dispatch from an aborting arm"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect Bail (op out (-> Int64)) (op mark (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (+ (* 100 (handle Bail 0
                            ((out () t (Bail.mark))
                             (mark () t (resume t (+ t 3))))
                            (+ (Bail.out) 999)))
                   (O.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))
