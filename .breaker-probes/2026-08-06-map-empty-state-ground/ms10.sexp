(case "ms10 PURE control — lookup-fallback-insert chain on let-bound Map.empty, no effects"
  (input  (do
            (def (main (: n Int64))
              (let ((m Map.empty))
                (let ((cur (match (Map.lookup m "k") ((Some x) x) ((None _u) 0))))
                  (Map.len (Map.insert m "k" (+ cur n))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))
