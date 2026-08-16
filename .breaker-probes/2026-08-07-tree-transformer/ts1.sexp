(case "ts1 a recursive TREE as a transformer op — crosses IN, the arm wraps it, crosses back OUT"
  (input  (do
            (type Tree (Leaf Int64) (Node (Tuple Tree Tree)))
            (effect St (op grow (-> Tree Tree)))
            (def (sum-t (: t Tree))
              (match t
                ((Tree.Leaf v) v)
                ((Tree.Node p) (match p ((tuple l r) (+ (sum-t l) (sum-t r)))))))
            (def (main (: n Int64))
              (handle St 0
                ((grow (t) s (resume (Tree.Node (tuple t (Tree.Leaf 10))) s)))
                (sum-t (St.grow (Tree.Node (tuple (Tree.Leaf n) (Tree.Leaf 7)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 22 Int64)))
