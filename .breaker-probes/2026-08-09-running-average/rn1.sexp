(case "rn1 a RUNNING-AVERAGE state (sum,count) — three feeds then a truncating divide read, negative total exercises toward-zero"
  (input  (do
            (effect E (op feed (-> Int64 Int64)) (op avg (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple 0 0)
                ((feed (x) s (match s
                               ((tuple tot cnt) (resume x (tuple (+ tot x) (+ cnt 1))))))
                 (avg () s (match s
                             ((tuple tot cnt) (resume (+ (* 10 (/ tot cnt)) cnt) s)))))
                (do (E.feed n) (E.feed (+ n 6)) (E.feed 3)
                    (E.avg))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 53 Int64))
  (call   main (: 0 Int64)) (output (: 33 Int64))
  (call   main (: -9 Int64)) (output (: -27 Int64)))
