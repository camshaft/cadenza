(case "xhsMultiCtrl the inline control for the merged multi-slot probe — the binder inlined at every use"
  (input  (do
            (effect T (op put (-> Int64 Int64 Int64)))
            (effect I (op step (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle T (: (Map.empty) (Map Int64 Int64))
                ((put (k v) m
                  (match (Map.lookup m k)
                    ((Some x) (resume x (Map.insert m k v)))
                    ((None) (resume (: 99 Int64) (Map.insert m k v))))))
                (handle I (: 0 Int64)
                  ((step (x) col
                    (resume (+ (* (+ col (+ x (% n 3))) 10)
                               (% (T.put (+ col (+ x (% n 3))) (* (+ col (+ x (% n 3))) 2)) 10))
                            (+ col (+ x (% n 3))))))
                  (let ((a (I.step (: 3 Int64))))
                    (let ((b (I.step (: 5 Int64))))
                      (let ((r (T.put (: 4 Int64) (: 7 Int64))))
                        (+ (* 1000 (+ (* 1000 a) b)) r)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 49109008 Int64))
  (call   main (: 0 Int64)) (output (: 39089099 Int64)))
