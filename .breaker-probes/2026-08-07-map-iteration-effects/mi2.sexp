(case "mi2 SORTED Set enumeration survives a draw-keyed insert — positional reads, the collision row exposes the missing third slot"
  (input  (do
            (effect Sx (op add (-> Int64 Int64)) (op dump (-> (List Int64))))
            (def (at-or (: xs (List Int64)) (: i Int64))
              (match (List.at xs i) ((Some v) v) ((None) -1)))
            (def (main (: n Int64))
              (handle Sx (Set.of (list 20 8))
                ((add (v) s (resume (Set.len s) (Set.insert s v)))
                 (dump () s (resume (Set.to-list s) s)))
                (do
                  (Sx.add n)
                  (let ((xs (Sx.dump)))
                    (+ (* 100 (at-or xs 0)) (+ (* 10 (at-or xs 1)) (at-or xs 2)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 600 Int64))
  (call   main (: 30 Int64)) (output (: 1030 Int64))
  (call   main (: 8 Int64)) (output (: 999 Int64)))
