(case "lf1 a recursive index-walk over a DRAW-BUILT list, scaled by an earlier draw — the walk itself is pure"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (sum-scaled (: xs (List Int64)) (: i Int64) (: k Int64))
              (match (List.at xs i)
                ((Some v) (+ (* v k) (sum-scaled xs (+ i 1) k)))
                ((None) 0)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (let ((k (St.next)))
                  (let ((xs (list (St.next) (St.next) (St.next))))
                    (sum-scaled xs 0 k)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 105 Int64))
  (call   main (: 2 Int64)) (output (: 24 Int64)))
