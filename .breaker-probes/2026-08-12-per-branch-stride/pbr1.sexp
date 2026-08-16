(case "pbr1 PER-BRANCH resume with different value AND stride — the else branch jumps the state 50, flipping the next dispatch's route"
  (input  (do
            (effect S (op route (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S n
                ((route (v) s
                  (if (< v s)
                      (resume (* v 10) (+ s 1))
                      (resume (+ v 100) (+ s 50)))))
                (+ (S.route 3) (* 1000 (S.route 3)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 30030 Int64))
  (call   main (: 1 Int64)) (output (: 30103 Int64))
  (call   main (: 3 Int64)) (output (: 30103 Int64)))
