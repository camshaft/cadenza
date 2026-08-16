(case "ts2 the composed multibyte slice view equals its literal twin and keys a Map"
  (input  (do
            (def (main (: n Int64))
              (do
                (def rope (String.concat "aé∀" "bçd"))
                (def inner (match (String.slice rope 1 5)
                             ((Some outer) (match (String.slice outer 1 3) ((Some i) i) ((None _u) "")))
                             ((None _u) "")))
                (+ (* 10 (if (= inner "∀b") 1 0))
                   (match (Map.lookup (Map.insert Map.empty "∀b" 7) inner)
                     ((Some v) v) ((None _u) -1)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 17 Int64)))
