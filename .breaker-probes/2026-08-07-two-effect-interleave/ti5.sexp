(case "ti5 a FOREIGN outer perform inside the inner region — A advances inside B's body, the post-region draw sees it"
  (input  (do
            (effect A (op a (-> Int64)))
            (effect B (op b (-> Int64)))
            (def (main (: n Int64))
              (handle A n
                ((a () s (resume s (+ s 1))))
                (+ (handle B 50
                     ((b () t (resume t (* t 2))))
                     (let ((x (B.b)))
                       (+ x (A.a))))
                   (A.a))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 61 Int64))
  (call   main (: 0 Int64)) (output (: 51 Int64)))
