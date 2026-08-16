(case "th2 a MATCH-shaped arm with three resume sites (sum-dispatch, not if)"
  (input  (do
            (effect St (op class (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((class (v) s
                  (match (% v 3)
                    (0 (resume (* v 10) (+ s 1)))
                    (1 (resume v s))
                    (_ (resume (- 0 v) (+ s 100))))))
                (+ (St.class 6) (+ (St.class 7) (St.class n)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 62 Int64)))
