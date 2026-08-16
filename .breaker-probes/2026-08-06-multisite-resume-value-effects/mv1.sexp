(case "mv1 a multi-site arm whose RESUME VALUE on one branch is a pure helper call"
  (input  (do
            (effect St (op sift (-> Int64 Int64)))
            (def (triple (: x Int64)) (* x 3))
            (def (main (: n Int64))
              (handle St 0
                ((sift (v) s (if (> v 10) (resume (triple v) (+ s 1)) (resume 0 s))))
                (+ (St.sift 20) (+ (St.sift n) (St.sift 30)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 150 Int64)))
