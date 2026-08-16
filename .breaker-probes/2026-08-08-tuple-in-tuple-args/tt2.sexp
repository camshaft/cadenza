(case "tt2 NESTED-tuple resume values across two dispatches — outer destructured by match, inner read by PROJECTION"
  (input  (do
            (effect E (op snap (-> (Tuple Int64 (Tuple Int64 Int64)))))
            (def (main (: n Int64))
              (handle E n
                ((snap () s (resume (tuple s (tuple (* s 10) (+ s 1))) (+ s 2))))
                (match (E.snap)
                  ((tuple a inner)
                   (let ((b (. inner 0)))
                     (let ((c (. inner 1)))
                       (match (E.snap)
                         ((tuple d inner2)
                          (+ a (+ b (+ c (+ d (+ (. inner2 0) (. inner2 1))))))))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 146 Int64))
  (call   main (: 0 Int64)) (output (: 26 Int64)))
