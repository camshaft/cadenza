(case "se2 selector routing per RECURSIVE hop with positional weights — parity of the advancing state picks left or right each iteration"
  (input  (do
            (effect S (op pick (-> Int64)) (op left (-> Int64 Int64)) (op right (-> Int64 Int64)))
            (def (walk (: k Int64) (: acc Int64))
              (if (< k 1) acc
                (let ((sel (S.pick)))
                  (let ((r (if (= sel 0) (S.left k) (S.right k))))
                    (walk (- k 1) (+ (* 10 acc) r))))))
            (def (main (: n Int64))
              (handle S n
                ((pick () s (resume (% s 2) (+ s 1)))
                 (left (v) s (resume (+ v 100) s))
                 (right (v) s (resume (+ v 200) s)))
                (walk 4 0)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 125521 Int64))
  (call   main (: 1 Int64)) (output (: 216421 Int64)))
