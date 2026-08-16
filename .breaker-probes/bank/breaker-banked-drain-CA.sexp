(case "a @requires-guarded helper called inside a fused-match arm enforces per-branch"
  (doc    "Conditions × the match-fusion seam: `halfp` carries `@requires (> d 0)` and is called from
           BOTH arms of a match on a call result (`mk`, a fusion candidate) with arm-local arguments
           derived from the payload binder. The fused clone must carry the requires-check injection
           into each branch copy: k=10 → Hi arm, d=4 → 25; k=2 → Lo arm, d=2 → 50; k=6 → Hi arm,
           d=0 violates → the requires trap fires (unreachable). A clone that dropped the injected
           check on one branch would run 100/0 (a div trap with the WRONG provenance) or worse fold
           it; a clone that duplicated the check into the untaken branch would trap spuriously.")
  (input  (do
            (type Sz (Hi Int64) (Lo Int64))
            (def (mk x) (if (> x 5) (Hi x) (Lo x)))
            (@ (requires (> d 0))
               (def (halfp (: d Int64)) (/ 100 d)))
            (def (main (: k Int64))
              (match (mk k)
                ((Hi h) (halfp (- h 6)))
                ((Lo w) (halfp w))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 25 Int64))
  (call   main (: 2 Int64)) (output (: 50 Int64))
  (call   main (: 6 Int64)) (trap "unreachable"))
