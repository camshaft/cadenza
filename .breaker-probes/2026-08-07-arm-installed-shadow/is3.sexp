(case "is3 the ARM's NEXT-STATE is computed by a nested SAME-effect handle — the shadow feeds the outer state thread"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (handle St (+ s 100)
                                        ((next () t (resume t (* t 2))))
                                        (+ (St.next) (St.next))))))
                (+ (St.next) (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 320 Int64))
  (call   main (: 0 Int64)) (output (: 300 Int64)))
