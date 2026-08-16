(case "a THREE-deep nested record update via chained with rebuilds only the spine"
  (doc    "The depth-3 + persistence upgrade of the two-level nested-with pin (:346 updates pos.y and
           reads only the UPDATED path): three chained `Record.with`s rebuild the a→b→c spine, and the
           reads check all four faces at once — the updated leaf through the new spine (1000s: 70),
           the ORIGINAL record's leaf UNCHANGED (100s: 7 — persistence; an in-place write through the
           projected sub-record corrupts it), the untouched SIBLING x sharing the deepest node (10s: 1
           — the rebuild must copy only the spine, and the copied node must keep its other fields),
           and the untouched top-level z (1s: 3) → 70713. The record twin of the RRB depth-3
           path-copy pin.")
  (input  (do
            (def (main (: d Int64))
              (let ((r (record (a (record (b (record (c d) (x 1))) (y 2))) (z 3))))
                (let ((r2 (Record.with r #"a"
                            (Record.with (. r a) #"b"
                              (Record.with (. (. r a) b) #"c" (* d 10))))))
                  (+ (* 1000 (. (. (. r2 a) b) c))
                     (+ (* 100 (. (. (. r a) b) c))
                        (+ (* 10 (. (. (. r2 a) b) x)) (. r2 z)))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 70713 Int64)))
