(case "rt3 round-tripped string keys a Map like the pre-trip original (identity through the byte world)"
  (input  (do
            (def (main (: k Int64))
              (do
                (def s (String.concat "ké" (if (> k 0) "y" "z")))
                (def s2 (Option.expect (String.from-bytes (String.to-bytes s)) "valid"))
                (match (Map.lookup (Map.insert Map.empty s 42) s2)
                  ((Some v) v) ((None _u) -1))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 42 Int64)))
