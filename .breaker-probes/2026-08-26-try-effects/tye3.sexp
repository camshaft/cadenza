(case "tye3 CONTROL: a `?` over a plain RUNTIME Result param (no effects)"
  (input (do
    (def (get (: r (Result Int64 String)))
      (Ok (+ (try r) 1)))
    (def (main (: n Int64))
      (match (get (if (> n 0) (Ok n) (Err "neg")))
        ((Ok v) v)
        ((Err _) -1)))
    (export main)))
  (call main (: 41 Int64)) (output (: 42 Int64)))
