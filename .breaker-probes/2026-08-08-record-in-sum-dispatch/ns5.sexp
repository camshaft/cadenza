(case "ns5 the nested two-layer match FOLDS when the scrutinee is a pure literal — only op-built scrutinees push it off the fold"
  (input  (do
            (type Box (Wrap (Record (: x Int64) (: y Int64))))
            (type Big (Node (Record (: tag Int64) (: inner Box))))
            (effect E (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((probe () s (resume s s)))
                (match (Big.Node (record (tag n) (inner (Box.Wrap (record (x 1) (y 2))))))
                  ((Big.Node outer)
                    (match (. outer inner)
                      ((Box.Wrap r) (+ (* 100 (. outer tag)) (+ (* 10 (. r x)) (. r y)))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 312 Int64))
  (call   main (: 0 Int64)) (output (: 12 Int64))
  (call   main (: -4 Int64)) (output (: -388 Int64)))
