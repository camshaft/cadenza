(case "ti1 A-B-A-B interleaved draws in ONE expression — each effect threads its own state independently"
  (input  (do
            (effect A (op a (-> Int64)))
            (effect B (op b (-> Int64)))
            (def (main (: n Int64))
              (handle A n
                ((a () s (resume s (+ s 1))))
                (handle B (* n 10)
                  ((b () t (resume t (+ t 100))))
                  (+ (A.a) (+ (B.b) (+ (A.a) (B.b)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 211 Int64))
  (call   main (: 1 Int64)) (output (: 123 Int64)))
