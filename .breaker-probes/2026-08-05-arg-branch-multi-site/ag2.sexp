(case "ag2 the branch reads a LOCAL derived from the op argument (mod-3 gate), two sites, different states"
  (input  (do
            (effect St (op sift (-> Int64 Int64)) (op count (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((sift (v) s (let ((d (% v 3))) (if (= d 0) (resume 1 (+ s 1)) (resume 0 s))))
                 (count (u) s (resume s s)))
                (+ (St.sift n) (+ (St.sift 4) (+ (St.sift 9) (* 1000 (St.count)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 2002 Int64)))
