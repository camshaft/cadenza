(case "g1 grafting through a DAG-shared subtree rebuilds one path and leaves the shared original intact"
  (input  (do
            (type Tree (Leaf Int64) (Node Tree Tree))
            (def (sum (: t Tree))
              (match t
                ((Leaf v) v)
                ((Node l r) (+ (sum l) (sum r)))))
            (def (graft (: t Tree) (: sub Tree))
              (match t
                ((Leaf _v) sub)
                ((Node l r) (Node (graft l sub) r))))
            (def (main (: k Int64))
              (let ((shared (Node (Leaf k) (Leaf (+ k 1)))))
                (let ((t (Node shared shared)))
                  (let ((g (graft t (Leaf 100))))
                    (+ (sum g) (* 1000 (sum t)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 22117 Int64)))
