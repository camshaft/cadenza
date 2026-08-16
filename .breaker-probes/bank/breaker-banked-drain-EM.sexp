(case "a user trap in ONE fused-match arm fires only on that branch"
  (doc    "The user-TRAP face of the fused-clone seam: one arm computes (Hi → h·10), the other arm's
           body is an unconditional `(trap …)` — the validation-reject idiom. Fusion clones BOTH
           bodies into the callee's branches, and the trap must fire ONLY when its branch is taken
           (k=7 → 70; k=2 → trap): a clone that hoisted the trap above the dispatch (or a
           trap-freedom analysis that treated the cloned trap as blocking the whole match) breaks
           the computing branch; a dropped trap arm silently returns garbage on the low path. The
           trap companion of the fused abort/perform arm pins (those exit via handlers; this is the
           unconditional divergence face).")
  (input  (do
            (type Sz (Hi Int64) (Lo Int64))
            (def (mk x) (if (> x 5) (Hi x) (Lo x)))
            (def (main (: k Int64))
              (match (mk k)
                ((Hi h) (* h 10))
                ((Lo w) (trap "low value not allowed"))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 70 Int64))
  (call   main (: 2 Int64)) (trap "unreachable"))
