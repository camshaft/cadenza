(case "an abortive perform in a recursive callee exits ONLY its own handle under an enclosing handler"
  (doc    "The adv-52 WORKING perimeter at the nested-handler face: `go` self-recurses to the
           abortive `Mx.bail 5` with NO pending continuation in the inner handle body — the abort
           yields 500 as the INNER handle's value, and the enclosing (different-effect) `A` handler
           is untouched — the outer handle passes the value through (500). Boxes adv-52 from the
           nesting side: the abort's non-local exit must stop at ITS handler even when the recursive
           specialization runs under a second enclosing frame (an exit that unwound one frame too
           far discards the outer handle's identity; one frame too few is the adv-52 resume bug —
           which fires only with a pending continuation, absent here). Guards the working face while
           the adv-52 fix lands.")
  (input  (do
            (effect A (op tick (-> Unit Int64)))
            (effect Mx (op bail (-> Int64 Int64)))
            (def (go (: n Int64)) (if (= n 0) (Mx.bail 5) (go (- n 1))))
            (def (main)
              (handle A 0 ((tick (u) s (resume s s)))
                (handle Mx 0 ((bail (v) s (* v 100)))
                  (go 2))))
            (export main)))
  (call   main) (output (: 500 Int64)))
