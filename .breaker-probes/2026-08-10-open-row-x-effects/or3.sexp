(case "or3 a RECORD handler state projected open-row inside the arm — the arm's row instantiation is independent of the body's"
  (input  (do
            (effect St (op bump (-> Int64 Int64)))
            (def (get-x r) (. r x))
            (def (main (: n Int64))
              (handle St (record (= x n) (= hits 0))
                ((bump (a) s (resume (+ (get-x s) a)
                                     (record (= x (+ (get-x s) a)) (= hits (+ (. s hits) 1))))))
                (let ((b1 (St.bump 10)))
                  (let ((b2 (St.bump 100)))
                    (+ b1 (* 1000 b2))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 113013 Int64))
  (call   main (: 0 Int64)) (output (: 110010 Int64)))
