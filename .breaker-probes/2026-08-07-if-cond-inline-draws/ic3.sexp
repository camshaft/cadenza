(case "ic3 CHAINED if-else-if where each condition draws — three rows land in three different arms"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 3))))
                (if (> (St.next) 10)
                    111
                    (if (> (St.next) 5)
                        (+ 200 (St.next))
                        (- 0 (St.next))))))
            (export main)))
  (call   main (: 11 Int64)) (output (: 111 Int64))
  (call   main (: 4 Int64)) (output (: 210 Int64))
  (call   main (: 0 Int64)) (output (: -6 Int64)))
