(case "dsc1 a DISCARDED DIVISION NEVER TRAPS — the do-interior quotient is computed for no one and the zero divisor that would trap in value position is ELIDED with the rest of the dead expression, both seeds returning the constant tail unchanged, pinning that a pure trapping form in discard position is droppable (the CDZ0307 contract: a valueless pure form has no effect and a trap is not an effect)"
  (input  (do
            (def (main (: n Int64))
              (do (/ 60 (% n 3)) (: 7 Int64)))
            (export main)))
  (call   main (: 10 Int64)) (output (: 7 Int64))
  (call   main (: 0 Int64)) (output (: 7 Int64)))
