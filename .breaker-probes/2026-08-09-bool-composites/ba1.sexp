(case "ba1 XOR composed from and/or/not over two draw comparisons — all three truth patterns reachable"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (let ((a (> (E.next) 2)))
                  (let ((b (> (E.next) 2)))
                    (+ (if (and (or a b) (not (and a b))) 100 200)
                       (- (E.probe) n))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 202 Int64))
  (call   main (: 1 Int64)) (output (: 202 Int64))
  (call   main (: 2 Int64)) (output (: 102 Int64)))
