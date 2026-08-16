(case "rh2 two-site arm over a PURE in-program effect nested inside a host block (no interpose)"
  (input  (do
            (effect ask (op get (-> Int64 Int64)))
            (effect St (op sift (-> Int64 Int64)))
            (def (main)
              (host (ask)
                (+ (ask.get 3)
                   (handle St 0
                     ((sift (v) s (if (> v 1) (resume v (+ s 1)) (resume 0 s))))
                     (+ (St.sift 5) (St.sift 1))))))
            (export main)))
  (host-responses (respond ask.get (: 30 Int64)))
  (host-calls (call ask.get))
  (output (: 35 Int64)))
