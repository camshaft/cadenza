(case "two same-named parameters in one def are a linearity error"
  (doc    "Pattern linearity at the PARAM LIST (finding #47, adv-47): `(def (f (: x Int64) (: x
           Int64)) x)` binds the name x twice in one binding position — the same CDZ0102 a name
           appearing twice in a flat pattern gets (Patterns Compose: the whole pattern MUST remain
           linear). Was an ML-front-end divergence (cadenza-ml silently let the second x shadow the
           first and RAN to the second argument's value while rcdzc rejected — fixed by the B9
           param-name-of Option + all-defs-well-scoped enforcement, self-host pins sread-eval:360/370);
           now both paths reject. Uniform ×3 targets.")
  (input  (do
            (def (f (: x Int64) (: x Int64)) x)
            (def (main) (f 1 2))
            (export main)))
  (call   main)
  (error  CDZ0102))
