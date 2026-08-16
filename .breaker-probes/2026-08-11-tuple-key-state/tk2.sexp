(case "tk2 the tuple KEY is built from the STATE COUNTER inside the arm — the key components come from the thread itself"
  (input  (do
            (effect S (op mark (-> Int64)) (op check (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle S (tuple n Map.empty)
                ((mark () st (match st ((tuple c m)
                    (resume (Map.len m) (tuple (+ c 1) (Map.insert m (tuple c (+ c 1)) 99))))))
                 (check (x y) st (match st ((tuple _c m)
                    (resume (match (Map.lookup m (tuple x y)) ((Some v) v) ((None _u) -1)) st)))))
                (let ((_a (S.mark)))
                  (+ (* 100 (S.check n (+ n 1))) (S.check (+ n 1) n)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 9899 Int64))
  (call   main (: 0 Int64)) (output (: 9899 Int64)))
