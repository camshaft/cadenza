(case "rv2 a row-op DERIVED record stored at depth is retrieved and destructured intact"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 (Record (x Int64) (y Int64)))))
              (if (= i 0) m
                (fill (- i 1) (Map.insert m i (Record.without (Record.extend (record (x i)) #"y" (* i 3)) ())))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (match (Map.lookup m 20)
                  ((Some r) (match r ((record (x a) (y b)) (+ (* 100 a) b))))
                  ((None _u) -1))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 2060 Int64)))
