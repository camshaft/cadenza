(case "sg2 the guard calls a PURE def over payload AND state — the nearness verdict flips as the state walks past the payload"
  (input  (do
            (effect S (op route (-> Int64 Int64)))
            (def (near (: a Int64) (: b Int64)) (< (if (< a b) (- b a) (- a b)) 3))
            (def (main (: n Int64))
              (handle S n
                ((route (v) s
                  (match v
                    ((guard x (near x s)) (resume 1 (+ s 1)))
                    (_other (resume 0 (+ s 1))))))
                (+ (S.route 6) (+ (* 10 (S.route 6)) (* 100 (S.route 6))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 111 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))
