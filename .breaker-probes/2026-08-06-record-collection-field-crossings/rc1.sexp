(case "rc1 a record with a LIST field crosses resume — the body projects and folds the collection field"
  (input  (do
            (effect St (op page (-> Int64 (Record (total Int64) (items (List Int64))))))
            (def (main (: n Int64))
              (handle St 0
                ((page (k) s (resume (record (total (* k 10)) (items (list k (+ k 1) (+ k 2)))) s)))
                (let ((r (St.page n)))
                  (+ (. r total)
                     (+ (List.len (. r items))
                        (match (List.at (. r items) 2) ((Some v) v) ((None _u) -1)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 60 Int64)))
