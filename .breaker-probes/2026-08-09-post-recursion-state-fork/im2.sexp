(case "im2 a walk with TWO exit conditions — divisibility of the draw OR a step cap, whichever fires first"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (walk (: steps Int64))
              (let ((d (E.next)))
                (if (or (= (% d 7) 0) (>= (+ steps 1) 5))
                    (+ (* 100 d) (* 10 (+ steps 1)))
                    (walk (+ steps 1)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (+ (walk 0) (- (E.probe) n))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 733 Int64))
  (call   main (: 12 Int64)) (output (: 1433 Int64))
  (call   main (: 8 Int64)) (output (: 1255 Int64)))
