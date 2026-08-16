(case "dg1 a DAG: one shared list inside THREE containers, mutated via one path, others intact"
  (input  (do
            (def (main (: n Int64))
              (do
                (def shared (list n 2))
                (def in-map (Map.insert Map.empty 1 shared))
                (def in-set-tup (tuple 9 shared))
                (def grown (List.push shared 99))
                (+ (* 1000 (List.len grown))
                   (+ (* 100 (match (Map.lookup in-map 1) ((Some xs) (List.len xs)) ((None _u) -1)))
                      (+ (* 10 (List.len (. in-set-tup 1)))
                         (List.len shared))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 3222 Int64)))
