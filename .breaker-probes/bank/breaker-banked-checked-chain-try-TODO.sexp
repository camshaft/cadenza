(case "a checked-arithmetic chain threads through try to an Option boundary at runtime operands"
  (input  (do
            (def (safe-fma (: a Int64) (: b Int64) (: c Int64))
              (let ((p (try (Int64.checked-mul a b))))
                (let ((s (try (Int64.checked-add p c))))
                  (Some s))))
            (def (main (: a Int64))
              (match (safe-fma a 3 100)
                ((Some v) v)
                ((None u) -1)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 115 Int64))
  (call   main (: 9223372036854775807 Int64)) (output (: -1 Int64)))
