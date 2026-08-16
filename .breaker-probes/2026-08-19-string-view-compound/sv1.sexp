(case "sv1 a multibyte seam-crossing String slice inside a TUPLE key matches the literal compound key"
  (input  (do
            (def (main (: n Int64))
              (do
                (def rope (String.concat "aé∀" "bçd"))
                (def w (match (String.slice rope 1 4)
                         ((Some v) v) ((None _u) "")))
                (match (Map.lookup (Map.insert Map.empty (tuple "é∀b" 1) 42) (tuple w 1))
                  ((Some v) v) ((None _u) -1))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 42 Int64)))
