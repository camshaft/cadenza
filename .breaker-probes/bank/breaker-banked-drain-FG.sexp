(case "three-level nested records compare structurally with scrambled field order at every depth"
  (doc    "The canonical-field-order equality walk at DEPTH 3, with the field order WRITTEN
           differently at EVERY level: outer `(outer …)(tag 9)` vs `(tag 9)(outer …)`, mid
           unchanged, inner `(v n)(w 2)` vs `(w 2)(v n)` — all three levels scramble, and structural
           `=` must canonicalize (sort fields) at each depth to see them equal → 1. The pinned
           order-independence :1082 is ONE level; a walk that canonicalized only the top record (or
           compared written order at depth) would call these unequal. Runtime n threads the deepest
           leaf so nothing const-folds the comparison.")
  (input  (do
            (def (main (: n Int64))
              (if (= (record (outer (record (mid (record (v n) (w 2))))) (tag 9))
                     (record (tag 9) (outer (record (mid (record (w 2) (v n))))))) 1 0))
            (export main)))
  (call   main (: 7 Int64)) (output (: 1 Int64)))
