(case "sos1 a SET-OF-STRINGS state with ARM-BUILT rope keys — the concat-built key dedups structurally against the FLAT seed literal, parity routes which inserts are no-ops"
  (input  (do
            (effect S (op tag (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (Set.of (list "k-e"))
                ((tag (v) st
                  (let ((key (String.concat "k" (if (= (% v 2) 0) "-e" "-o"))))
                    (let ((s2 (Set.insert st key)))
                      (resume (Set.len s2) s2)))))
                (let ((a (S.tag n)))
                  (let ((b (S.tag (+ n 1))))
                    (let ((c (S.tag 4)))
                      (+ (* 10 (+ (* 10 a) b)) c))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 122 Int64))
  (call   main (: 3 Int64)) (output (: 222 Int64)))
