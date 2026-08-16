(case "ha1 a SINGLE-site in-program handle nested inside a host block (skew minimal)"
  (input  (do
            (effect ask (op get (-> Int64 Int64)))
            (effect St (op bump (-> Unit Int64)))
            (def (main)
              (host (ask)
                (+ (ask.get 3)
                   (handle St 0
                     ((bump (u) s (resume s (+ s 1))))
                     (+ (St.bump) (St.bump))))))
            (export main)))
  (host-responses (respond ask.get (: 30 Int64)))
  (host-calls (call ask.get))
  (output (: 31 Int64)))
