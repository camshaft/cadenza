(case "checked-add over runtime operands computes Some or None by range"
  (input  (do
            (def (main (: a Int64) (: b Int64))
              (match (Int64.checked-add a b)
                ((Some v) v)
                ((None u) -1)))
            (export main)))
  (call   main (: 20 Int64) (: 22 Int64)) (output (: 42 Int64))
  (call   main (: 9223372036854775807 Int64) (: 1 Int64)) (output (: -1 Int64)))
