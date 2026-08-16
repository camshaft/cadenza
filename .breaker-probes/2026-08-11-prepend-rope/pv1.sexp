(case "pv1 the rope state grows by PREPEND — the recursive walk builds digit(1)..digit(n) left-to-right, first byte pins the order"
  (input  (do
            (effect S (op add (-> Int64 Int64)) (op dump (-> String)))
            (def (walk (: k Int64))
              (if (< k 1) 0 (let ((_d (S.add k))) (walk (- k 1)))))
            (def (digit (: v Int64))
              (if (= v 1) "a" (if (= v 2) "b" (if (= v 3) "c" "d"))))
            (def (first-byte (: t String))
              (match (Bytes.at (String.to-bytes t) 0) ((Some b) b) ((None _u) -1)))
            (def (main (: n Int64))
              (handle S ""
                ((add (v) s (resume 0 (String.concat (digit v) s)))
                 (dump () s (resume s s)))
                (let ((_w (walk n)))
                  (let ((t (S.dump)))
                    (+ (* 100 (String.byte-len t)) (first-byte t))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 397 Int64))
  (call   main (: 1 Int64)) (output (: 197 Int64)))
