(case "smk1 a STRING-KEYED Map state whose keys are BUILT IN THE ARM — parity routes each value to a concat-computed bucket, accumulate-or-insert answers the bucket total"
  (input  (do
            (effect S (op tag (-> String Int64 Int64)))
            (def (main (: n Int64))
              (handle S Map.empty
                ((tag (pre v) m
                  (let ((key (String.concat pre (if (= (% v 2) 0) "-e" "-o"))))
                    (let ((total (match (Map.lookup m key)
                                   ((Some x) (+ x v))
                                   ((None u) v))))
                      (resume total (Map.insert m key total))))))
                (let ((a (S.tag "a" n)))
                  (let ((b (S.tag "a" (+ n 2))))
                    (let ((c (S.tag "b" (+ n 1))))
                      (+ (* 10000 a) (+ (* 100 b) c)))))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 41005 Int64))
  (call   main (: 7 Int64)) (output (: 71608 Int64)))
