(case "cxm minimal: one Char as a map VALUE"
  (input  (do
            (def (main (: n Int64))
              (match (Map.lookup (Map.insert Map.empty 1 #\a) 1)
                ((Some c) (if (= c #\a) 1 0))
                ((None _u) -1)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
