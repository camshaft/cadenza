(example
  (id "binary-search-tree")
  (name "Binary search tree")
  (theme "data-and-collections")
  (surface "sexpr")
  (source (do
  (type Tree (Leaf Unit) (Node (Tuple Int64 Tree Tree)))

  (def
    (insert (: t Tree) (: x Int64))
    (match
      t
      ((Leaf _u) (Node #tuple(x (Leaf unit) (Leaf unit))))
      ((Node nd)
        (match
          nd
          (#tuple(v l r)
            (if
              (< x v)
              (Node #tuple(v (insert l x) r))
              (if (> x v) (Node #tuple(v l (insert r x))) (Node #tuple(v l r)))))))))

  (def
    (inorder (: t Tree))
    (match
      t
      ((Leaf _u) (: #list() (List Int64)))
      ((Node nd)
        (match nd (#tuple(v l r) ((. List concat) ((. List push) (inorder l) v) (inorder r)))))))

  (def
    (build (: xs (List Int64)) (: i Int64) (: t Tree))
    (if
      (= i ((. List len) xs))
      t
      (match
        ((. List at) xs i)
        ((Some x) (build xs (+ i 1) (insert t x)))
        ((None) (trap "build: index out of range")))))

  (def (main) (let ((xs #list(5 3 8 1 4 7 9 2 6))) (inorder (build xs 0 (Leaf unit)))))

  (export main)))
  (expected (: #list(1 2 3 4 5 6 7 8 9) (List Int64))))
