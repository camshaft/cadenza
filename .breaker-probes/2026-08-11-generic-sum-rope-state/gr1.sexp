(case "gr1 a user GENERIC sum with a ROPE payload as handler state — Hole-to-Full transition and payload byte-len across dispatch"
  (input  (do
            (effect S (op wrap (-> Int64 Int64)) (op peek (-> Int64)))
            (type (Box a) (Full a) (Hole))
            (def (main (: n Int64))
              (handle S (Hole)
                ((wrap (v) st
                  (resume (match st ((Full _s) 1) ((Hole) 0))
                          (Full (String.concat "p" "q"))))
                 (peek () st
                  (resume (match st
                            ((Full s) (String.byte-len s))
                            ((Hole) -1))
                          st)))
                (let ((a (S.peek)))
                  (let ((_b (S.wrap n)))
                    (+ (* 10 (S.peek)) a)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 19 Int64))
  (call   main (: 0 Int64)) (output (: 19 Int64)))
