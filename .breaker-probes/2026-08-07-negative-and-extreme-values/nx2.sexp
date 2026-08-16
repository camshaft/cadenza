(case "nx2 a 2^62 seed threads exactly — the difference of consecutive draws recovers the stride"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St 4611686018427387904
                ((next () s (resume s (- s n))))
                (- (St.next) (St.next))))
            (export main)))
  (call   main (: 1000000007 Int64)) (output (: 1000000007 Int64))
  (call   main (: 1 Int64)) (output (: 1 Int64)))
