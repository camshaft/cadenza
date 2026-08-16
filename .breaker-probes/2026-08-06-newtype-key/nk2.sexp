(case "nk2 a newtype over a HEAP payload (rope string) keys by content through the nominal layer"
  (input  (do
            (type Label (Label String))
            (def (main (: k Int64))
              (match (Map.lookup (Map.insert Map.empty (Label "hello") 9)
                                 (Label (String.concat "hel" (if (> k 0) "lo" "p"))))
                ((Some v) v) ((None _u) -1)))
            (export main)))
  (call   main (: 1 Int64)) (output (: 9 Int64))
  (call   main (: 0 Int64)) (output (: -1 Int64)))
