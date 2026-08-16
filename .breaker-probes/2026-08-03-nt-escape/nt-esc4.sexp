(case "nt-esc4 DES-style Duration face: param'd newtype escape loses the nominal label"
  (input  (do
            (type Duration (Duration UInt64))
            (def (secs (: n UInt64)) (Duration.Duration (* n 1000000000)))
            (def (main (: n UInt64)) (secs n))
            (export main)))
  (call   main (: 5 UInt64)) (output (: 5000000000 Duration)))
