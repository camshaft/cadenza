(case "za3 NESTED literal-arm dispatch — each level matches a let-bound draw, the innermost arm reads a third"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 3))))
                (let ((d1 (St.next)))
                  (match d1
                    (5 (let ((d2 (St.next)))
                         (match d2
                           (8 (+ 100 (St.next)))
                           (_i (- 0 _i)))))
                    (_o (* 10 _o))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 111 Int64))
  (call   main (: 3 Int64)) (output (: 30 Int64)))
