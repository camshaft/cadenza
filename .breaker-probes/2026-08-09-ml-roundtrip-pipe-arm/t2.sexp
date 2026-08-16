(case "t2 shl in arm"
  (input  (do
            (effect E (op tag (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((tag (x) s (resume (<< x 2) (+ s 1))))
                (E.tag 3)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 12 Int64)))
