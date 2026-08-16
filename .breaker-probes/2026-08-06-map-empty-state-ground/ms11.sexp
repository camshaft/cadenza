(case "ms11 CONTROL Int64-keyed lookup-only on Map.empty — types coincide with the default"
  (input  (do
            (def (main (: n Int64))
              (let ((m Map.empty))
                (match (Map.lookup m 1) ((Some x) x) ((None _u) n))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64)))
