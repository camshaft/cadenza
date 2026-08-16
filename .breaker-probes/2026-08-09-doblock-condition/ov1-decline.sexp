(case "ov1 an if CONDITION that is a whole do-block — two draws then a comparison, the block's value routes the branch"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (+ (if (do (def a (E.next)) (def b (E.next)) (> (+ a b) 5)) 100 200)
                   (* 10 (- (E.probe) n)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 120 Int64))
  (call   main (: 0 Int64)) (output (: 220 Int64))
  (call   main (: 2 Int64)) (output (: 220 Int64)))
