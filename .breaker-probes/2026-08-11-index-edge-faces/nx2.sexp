(case "nx2 i64-EXTREME indices — MAX and MIN as List.at arguments both answer the None fallback; a truncating index marshal would wrap into range"
  (input  (do
            (effect S (op at (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (list 10 20 30)
                ((at (i) s (resume (match (List.at s i) ((Some v) v) ((None _u) -7)) s)))
                (+ (* 1000 (S.at 9223372036854775807))
                   (S.at -9223372036854775808))))
            (export main)))
  (call   main (: 0 Int64)) (output (: -7007 Int64)))
