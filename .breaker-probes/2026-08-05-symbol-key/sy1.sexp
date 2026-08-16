(case "sy1 a Symbol built from a ROPE string keys a Map like its literal-symbol twin"
  (input  (do
            (def (main (: k Int64))
              (match (Map.lookup (Map.insert Map.empty #"hello" 42)
                                 (Symbol.of (String.concat "hel" (if (> k 0) "lo" "p"))))
                ((Some v) v) ((None _u) -1)))
            (export main)))
  (call   main (: 1 Int64)) (output (: 42 Int64))
  (call   main (: 0 Int64)) (output (: -1 Int64)))
