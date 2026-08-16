(case "nk1 a scalar newtype computed via wrap/unwrap arithmetic keys a Map like its direct twin"
  (input  (do
            (type Meters (Meters Int64))
            (def (add-m (: a Meters) (: b Meters))
              (match a ((Meters x) (match b ((Meters y) (Meters (+ x y)))))))
            (def (main (: n Int64))
              (match (Map.lookup (Map.insert Map.empty (Meters 42) 7)
                                 (add-m (Meters n) (Meters 2)))
                ((Some v) v) ((None _u) -1)))
            (export main)))
  (call   main (: 40 Int64)) (output (: 7 Int64))
  (call   main (: 41 Int64)) (output (: -1 Int64)))
