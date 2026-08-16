(case "sb2 the built rope keys a Map like its literal twin (builder→key at multibyte density)"
  (input  (do
            (def (build (: i Int64) (: acc String))
              (if (= i 0) acc
                  (build (- i 1) (String.concat acc (if (= (% i 2) 0) "é" "x")))))
            (def (main (: n Int64))
              (match (Map.lookup (Map.insert Map.empty "éxéx" 42) (build n ""))
                ((Some v) v) ((None _u) -1)))
            (export main)))
  (call   main (: 4 Int64)) (output (: 42 Int64))
  (call   main (: 6 Int64)) (output (: -1 Int64)))
