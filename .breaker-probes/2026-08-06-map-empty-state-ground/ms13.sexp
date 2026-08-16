(case "ms13 PURE control — list-valued lookup-fallback then push (the ms6 arm body, no effects)"
  (input  (do
            (def (main (: n Int64))
              (let ((m Map.empty))
                (let ((xs (match (Map.lookup m "k") ((Some ys) ys) ((None _u) (list)))))
                  (let ((nxs (List.push xs n)))
                    (List.len nxs)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))
