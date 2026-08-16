(case "me2 two delegated effects via NESTED host blocks interleave in program order"
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect B (op get (-> Unit Int64)))
            (def (main (: k Int64))
              (host (A)
                (host (B)
                  (+ (A.get unit)
                     (+ (* 10 (B.get unit))
                        (* 100 (A.get unit)))))))
            (export main)))
  (host-responses (respond a.get (: 1 Int64)) (respond b.get (: 2 Int64)) (respond a.get (: 3 Int64)))
  (host-calls (call a.get) (call b.get) (call a.get))
  (call   main (: 0 Int64)) (output (: 321 Int64)))
