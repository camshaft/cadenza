(case "an effect reached only through one arm of a fused match still requires its grant"
  (doc    "The fused-arm face of the no-home check: `io.get` is performed ONLY in the Hi arm of a
           match on a call result (`mk` — a fusion candidate whose arms clone into the callee's
           branches), with no handler and no delegation. Reachability must find the perform through
           the fused/cloned arm and reject CDZ0401 — fusion is not a loophole that hides an arm's
           effect from the grant check (the untaken-at-this-argument Lo path doesn't excuse it; a
           syntactically reached effect requires a grant, per the statically-dead-branch pins). The
           match-fusion companion of the closure-through-HOF rejection (:148).")
  (input  (do
            (effect io (op get (-> Unit Int64)))
            (type Sz (Hi Int64) (Lo Int64))
            (def (mk x) (if (> x 5) (Hi x) (Lo x)))
            (def (main (: k Int64))
              (match (mk k)
                ((Hi h) (+ h (io.get)))
                ((Lo w) w)))
            (export main)))
  (call   main (: 2 Int64))
  (error  CDZ0401))
