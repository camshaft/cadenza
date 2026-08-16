(case "sg1 a PURE guard comparing the op payload to the STATE binder routes the arm — same payload lands differently as state advances"
  (input  (do
            (effect S (op route (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S n
                ((route (v) s
                  (match v
                    ((guard x (< x s)) (resume (* 100 x) (+ s 1)))
                    (_other (resume v (+ s 10))))))
                (+ (S.route 3) (* 1000 (S.route 3)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 300300 Int64))
  (call   main (: 0 Int64)) (output (: 300003 Int64)))
