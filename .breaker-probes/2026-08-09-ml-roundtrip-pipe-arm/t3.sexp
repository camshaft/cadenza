(case "t3 pipe-or in body under handle"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (| (E.next) 8)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 11 Int64)))
