(do
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
  (export main))
