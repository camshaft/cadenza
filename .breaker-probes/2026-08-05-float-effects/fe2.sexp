(case "fe2 float NaN-adjacent: abort arm computes 0.0/0.0-free but state crosses INF (overflow-adjacent float state)"
  (input  (do
            (effect St (op halt (-> Unit Float64)))
            (def (main)
              (handle St 1.0e308
                ((halt (u) s (+ s s)))
                (St.halt)))
            (export main)))
  (output (: inf Float64)))
