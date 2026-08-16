(case "rc2 a record with a SET field as op ARGUMENT — the arm probes the collection beside the scalar"
  (input  (do
            (effect St (op audit (-> (Record (want Int64) (seen (Set Int64))) Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((audit (r) s (resume (+ (* 100 (if (Set.contains (. r seen) (. r want)) 1 0))
                                         (Set.len (. r seen)))
                              s)))
                (St.audit (record (want n) (seen (Set.of (list 2 n 9)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 103 Int64)))
