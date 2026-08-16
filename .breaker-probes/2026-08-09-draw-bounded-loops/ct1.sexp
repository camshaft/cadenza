(case "ct1 the LOOP BOUND is itself a draw — a first dispatch sizes the walk, the walk then draws that many times"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (walk (: k Int64) (: acc Int64))
              (if (<= k 0) acc (walk (- k 1) (+ acc (E.next)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (+ (* 100 (walk (+ (% (E.next) 4) 1) 0))
                   (- (E.probe) n))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 1204 Int64))
  (call   main (: 0 Int64)) (output (: 102 Int64))
  (call   main (: 7 Int64)) (output (: 3805 Int64)))
