(case "pc3 the interleaved middle op SERVES when made two-site itself"
  (input  (do
            (effect St (op sift (-> Int64 Int64)) (op peek (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s)))
                 (peek (u) s (if (> s 0) (resume s s) (resume -1 s))))
                (+ (St.sift 20) (+ (St.peek) (St.sift 30)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 51 Int64)))
