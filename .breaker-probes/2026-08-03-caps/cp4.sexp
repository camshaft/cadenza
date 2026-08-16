(case "cp4 a handled effect SHADOWS delegation for the same effect inside the handle"
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (def (main (: k Int64))
              (host (A)
                (+ (A.get unit)
                   (handle A 500 ((get (_u) s (resume s s)))
                     (A.get unit)))))
            (export main)))
  (host-responses (respond a.get (: 7 Int64)))
  (host-calls (call a.get))
  (call   main (: 0 Int64)) (output (: 507 Int64)))
