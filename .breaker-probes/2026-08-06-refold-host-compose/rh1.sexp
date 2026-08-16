(case "rh1 a two-site arm INTERPOSES a host effect (hit transforms, miss substitutes)"
  (input  (do
            (effect ask (op get (-> Int64 Int64)))
            (def (main)
              (host (ask)
                (handle ask unit
                  ((get (k) s (if (> k 0) (resume (+ (ask.get k) 1000) s) (resume -1 s))))
                  (+ (ask.get 3) (ask.get 0)))))
            (export main)))
  (host-responses (respond ask.get (: 30 Int64)))
  (host-calls (call ask.get))
  (output (: 1029 Int64)))
