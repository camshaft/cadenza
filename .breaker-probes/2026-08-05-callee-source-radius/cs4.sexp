(case "cs4 looked-up closure as op ARG where the LOOKUP KEY is perform-computed (both positions effectful, arm-side apply)"
  (input  (do
            (effect Ap (op app (-> (-> Int64 Int64) Int64)) (op pick (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def ops (Map.insert (Map.insert Map.empty 0 (fn ((: x Int64)) (* x 2))) 1 (fn ((: x Int64)) (+ x 1000))))
                (handle Ap n
                  ((app (f) s (resume (f s) (+ s 1)))
                   (pick (u) s (resume (% s 2) (+ s 1))))
                  (match (Map.lookup ops (Ap.pick))
                    ((Some g) (Ap.app g))
                    ((None _u) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1006 Int64)))
