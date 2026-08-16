(case "sk1 a SYMBOL-keyed Map state — parity routes each payload to the even or odd counter, interned identity survives the thread"
  (input  (do
            (effect S (op tag (-> Int64 Int64)) (op read (-> Int64)))
            (def (main (: n Int64))
              (handle S Map.empty
                ((tag (v) s
                  (let ((k (if (= (% v 2) 0) (Symbol.of "even") (Symbol.of "odd"))))
                    (resume (match (Map.lookup s k) ((Some c) c) ((None _u) 0))
                            (Map.insert s k (+ (match (Map.lookup s k) ((Some c) c) ((None _u) 0)) 1)))))
                 (read () s
                  (resume (+ (* 10 (match (Map.lookup s (Symbol.of "even")) ((Some c) c) ((None _u) 0)))
                             (match (Map.lookup s (Symbol.of "odd")) ((Some c) c) ((None _u) 0)))
                          s)))
                (let ((_a (S.tag 2)))
                  (let ((_b (S.tag 4)))
                    (let ((_c (S.tag n)))
                      (S.read))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 21 Int64))
  (call   main (: 6 Int64)) (output (: 30 Int64)))
