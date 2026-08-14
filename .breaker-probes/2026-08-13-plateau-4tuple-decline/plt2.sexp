(case "plt2 the LOOP-DRIVEN plateau tracker — the same three-let two-if arm that declines straight-line folds fine when one recursive driver walks the feed list"
  (input  (do
            (effect S (op feed (-> Int64 Int64)))
            (def (drive (: vals (List Int64)) (: i Int64) (: acc Int64))
              (match (List.at vals i)
                ((Some v) (drive vals (+ i 1) (+ (* acc 100) (S.feed v))))
                ((None u) acc)))
            (def (main (: n Int64))
              (handle S (tuple -999 0 0 -1)
                ((feed (v) st
                  (match st
                    ((tuple prev run bl bv)
                      (let ((r2 (if (= v prev) (+ run 1) 1)))
                        (let ((bl2 (if (> r2 bl) r2 bl)))
                          (let ((bv2 (if (> r2 bl) v bv)))
                            (resume (+ (* bl2 10) (% bv2 10))
                                    (tuple v r2 bl2 bv2)))))))))
                (drive (list 4 4 n n n) 0 0)))
            (export main)))
  (call   main (: 4 Int64)) (output (: 1424344454 Int64))
  (call   main (: 7 Int64)) (output (: 1424242437 Int64)))
