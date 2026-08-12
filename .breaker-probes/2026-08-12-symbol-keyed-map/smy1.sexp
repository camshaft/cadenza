(case "smy1 a SYMBOL-KEYED Map state — the op takes a Symbol and routes accumulation by interned identity, the same label from two different dispatches lands in one bucket"
  (input  (do
            (effect S (op acc (-> Symbol Int64 Int64)))
            (def (main (: n Int64))
              (handle S (: Map.empty (Map Symbol Int64))
                ((acc (k v) m
                  (let ((total (match (Map.lookup m k)
                                 ((Some x) (+ x v))
                                 ((None u) v))))
                    (resume total (Map.insert m k total)))))
                (let ((a (S.acc (Symbol.of "hot") n)))
                  (let ((b (S.acc (Symbol.of "cold") (+ n 1))))
                    (let ((c (S.acc (Symbol.of "hot") 10)))
                      (+ (* 10000 a) (+ (* 100 b) c)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 30413 Int64))
  (call   main (: 40 Int64)) (output (: 404150 Int64)))
