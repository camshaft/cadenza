(case "ms9 PURE control — Map.lookup on a let-bound Map.empty, no effects anywhere"
  (input  (do
            (def (main (: n Int64))
              (let ((m Map.empty))
                (match (Map.lookup m "k") ((Some x) x) ((None _u) n))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64)))
