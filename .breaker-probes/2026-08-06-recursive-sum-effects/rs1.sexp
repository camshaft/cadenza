(case "rs1 a RECURSIVE user sum (Tree) crosses resume; the body folds it"
  (input  (do
            (effect St (op grow (-> Int64 Tree)))
            (type Tree (Leaf Int64) (Node Tree Tree))
            (def (sum-tree t)
              (match t
                ((Tree.Leaf v) v)
                ((Tree.Node l r) (+ (sum-tree l) (sum-tree r)))))
            (def (main (: n Int64))
              (handle St 0
                ((grow (v) s (resume (Tree.Node (Tree.Leaf v) (Tree.Node (Tree.Leaf (* v 2)) (Tree.Leaf 1))) s)))
                (sum-tree (St.grow n))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 16 Int64)))
