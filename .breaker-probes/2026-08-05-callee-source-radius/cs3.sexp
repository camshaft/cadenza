(case "cs3 a collection-looked-up closure passed as the OP ARGUMENT (arm applies it to state)"
  (input  (do
            (effect Ap (op app (-> (-> Int64 Int64) Int64)))
            (def (main (: n Int64))
              (do
                (def ops (Map.insert (Map.insert Map.empty 0 (fn ((: x Int64)) (* x 2))) 1 (fn ((: x Int64)) (+ x 1000))))
                (handle Ap n
                  ((app (f) s (resume (f s) (+ s 1))))
                  (match (Map.lookup ops 1)
                    ((Some g) (Ap.app g))
                    ((None _u) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1005 Int64)))
