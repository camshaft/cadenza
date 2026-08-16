(case "gs5 the generic applied to ITSELF — (Container (Container Int64)) double-wraps and double-unwraps"
  (input  (do
            (type (Container a) (Full a))
            (def (unwrap2 (: b (Container (Container Int64))))
              (match b ((Full inner) (match inner ((Full v) (* 2 v))))))
            (def (main (: k Int64)) (unwrap2 (Full (Full k))))
            (export main)))
  (call   main (: 6 Int64)) (output (: 12 Int64))
  (call   main (: -5 Int64)) (output (: -10 Int64)))
