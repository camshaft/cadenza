(case "ag3 the branch condition reads the ARG AND the STATE together ((> v s)) — the compound face"
  (input  (do
            (effect St (op sift (-> Int64 Int64)) (op count (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((sift (v) s (if (> v s) (resume v (+ s 1)) (resume 0 s)))
                 (count (u) s (resume s s)))
                (+ (St.sift n) (+ (St.sift 0) (+ (St.sift 3) (* 1000 (St.count)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2008 Int64)))
