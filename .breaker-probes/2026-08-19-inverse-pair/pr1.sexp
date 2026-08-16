(case "pr1 a PAIR of mutually-referencing tries: ids->names and names->ids stay inverse under churn"
  (input  (do
            (def (build (: i Int64) (: fwd (Map Int64 Int64)) (: rev (Map Int64 Int64)))
              (if (= i 0) (tuple fwd rev)
                (build (- i 1) (Map.insert fwd i (+ i 1000)) (Map.insert rev (+ i 1000) i))))
            (def (check (: i Int64) (: fwd (Map Int64 Int64)) (: rev (Map Int64 Int64)) (: ok Int64))
              (if (= i 0) ok
                (check (- i 1) fwd rev
                  (+ ok (match (Map.lookup fwd i)
                          ((Some nm) (match (Map.lookup rev nm)
                                       ((Some back) (if (= back i) 1 0))
                                       ((None _u) 0)))
                          ((None _u) 0))))))
            (def (main (: n Int64))
              (match (build n Map.empty Map.empty)
                ((tuple fwd rev) (check n fwd rev 0))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 40 Int64)))
