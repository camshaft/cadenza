(case "ds3 a multibyte slice VIEW keys a Map found by its flat literal twin, and to-bytes matches"
  (input  (do
            (def (main (: a Int64))
              (do
                (def v (Option.expect (String.slice (String.concat "xé" (String.concat "日x" "😀")) a 3) "v"))
                (+ (* 10 (match (Map.lookup (Map.insert Map.empty "é日" 42) v) ((Some r) r) ((None _u) -1)))
                   (if (= (String.to-bytes v) (String.to-bytes "é日")) 1 0))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 421 Int64)))
