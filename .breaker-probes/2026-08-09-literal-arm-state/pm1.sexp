(case "pm1 LITERAL match arms grade the state INSIDE the handler arm — a mod-4 walker crosses all four literal rows"
  (input  (do
            (effect E (op tag (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 4)
                ((tag () s (resume (match s
                                     (0 70)
                                     (1 81)
                                     (2 92)
                                     (_ 63))
                                   (% (+ s 1) 4))))
                (+ (E.tag) (+ (* 10 (E.tag)) (* 100 (E.tag))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 10080 Int64))
  (call   main (: 2 Int64)) (output (: 7722 Int64))
  (call   main (: 3 Int64)) (output (: 8863 Int64)))
