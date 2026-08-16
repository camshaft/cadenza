(case "cp3 the SAME op name on a host-delegated effect and an in-program-handled one do not cross-talk"
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect B (op get (-> Unit Int64)))
            (def (main (: k Int64))
              (host (A)
                (+ (A.get unit)
                   (handle B 50 ((get (_u) s (resume s s)))
                     (B.get unit)))))
            (export main)))
  (host-responses (respond a.get (: 7 Int64)))
  (host-calls (call a.get))
  (call   main (: 0 Int64)) (output (: 57 Int64)))
