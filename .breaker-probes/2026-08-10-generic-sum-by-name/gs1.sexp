(case "gs1 a user-declared GENERIC sum resolves by NAME in a param annotation — the applied (Container Int64) checks and the payload unwraps"
  (input  (do
            (type (Container a) (Full a))
            (def (unwrap (: b (Container Int64))) (match b ((Full v) v)))
            (def (main (: k Int64)) (unwrap (Full k)))
            (export main)))
  (call   main (: 7 Int64)) (output (: 7 Int64))
  (call   main (: -12 Int64)) (output (: -12 Int64)))
