(case "q6 all FIVE arguments are draws — left-to-right evaluation order pinned at width five by distinct weights"
  (input  (do
            (effect E (op next (-> Int64))
                      (op quint (-> Int64 Int64 Int64 Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (quint (a b c d e) s
                  (resume (+ a (+ (* 2 b) (+ (* 3 c) (+ (* 4 d) (+ (* 5 e) s)))))
                          (+ s 1))))
                (E.quint (E.next) (E.next) (E.next) (E.next) (E.next))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 45 Int64))
  (call   main (: 3 Int64)) (output (: 93 Int64))
  (call   main (: -2 Int64)) (output (: 13 Int64)))
