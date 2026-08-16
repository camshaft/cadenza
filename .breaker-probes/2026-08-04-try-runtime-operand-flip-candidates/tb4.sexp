(case "tb4 control: the corpus get3 shape with a runtime index arg"
  (input  (do
            (def (pick (: xs (List Int64)) (: idx Int64))
              (let ((v (try (List.at xs idx))))
                (Some (* v 10))))
            (def (main (: n Int64))
              (match (pick (list 10 20 30) n)
                ((Some v) v)
                ((None _u) -1)))
            (export main)))
  (call   main (: 1 Int64)) (output (: 200 Int64)))
