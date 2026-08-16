(case "t1 pipe-or in arm"
  (input  (do
            (effect E (op tag (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((tag (x) s (resume (| x 8) (+ s 1))))
                (E.tag 3)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 11 Int64)))
