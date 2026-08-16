(case "px1 observed tail performer at depth 100 — values must agree while the stack face is under repair"
  (input  (do
            (effect Acc (op push (-> Int64 Int64)) (op size (-> Int64)))
            (def (grow (: n Int64))
              (if (< n 1) 0 (match (Acc.push n) (_ (grow (- n 1))))))
            (def (main (: n Int64))
              (handle Acc 0
                ((push (v) s (resume s (+ s 1)))
                 (size () s (resume s s)))
                (let ((g (grow n))) (+ (* 10 g) (Acc.size)))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 100 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64))
  (call   main (: 7 Int64)) (output (: 7 Int64)))
