(case "hc2 helpers at TWO depths both perform — the call-site draw, mid's draw, and leaf's draw arrive in order"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (leaf (: k Int64)) (+ (St.next) k))
            (def (mid (: k Int64)) (+ (* (St.next) 100) (leaf k)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (mid (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 612 Int64))
  (call   main (: 0 Int64)) (output (: 102 Int64)))
