(case "a stateful perform in an OR's right operand fires only when the left is false"
  (doc    "The or-connective face of conditional perform elision WITH state observation (the pinned
           abortive case :271 uses `and` + a non-resuming handler; here the handler RESUMES and the
           final `(Ctr.next)` reads how many performs ran): b=true → `or` short-circuits, the rhs
           perform NEVER fires, final next reads 0 → 10·1 + 0 = 10; b=false → rhs perform fires
           (reads 0 → false, 0 > 100 fails) so the `or` is false AND the state advanced → 10·0 + 1 =
           1. Both digits move together — a speculative rhs evaluation shows 11, a dropped rhs
           perform on the false path shows 0. Pins short-circuit exactness for a RESUMING stateful
           perform in `or`-rhs position, state-counted.")
  (input  (do
            (effect Ctr (op next (-> Unit Int64)))
            (def (main (: b Bool))
              (handle Ctr 0
                ((next (u) s (resume s (+ s 1))))
                (+ (* 10 (if (or b (> (Ctr.next) 100)) 1 0))
                   (Ctr.next))))
            (export main)))
  (call   main (: true Bool)) (output (: 10 Int64))
  (call   main (: false Bool)) (output (: 1 Int64)))
