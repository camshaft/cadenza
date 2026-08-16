(case "rs2 a recursive sum as op ARGUMENT — the arm dispatches on its shape"
  (input  (do
            (effect St (op weigh (-> Tree Int64)))
            (type Tree (Leaf Int64) (Node Tree Tree))
            (def (main (: n Int64))
              (handle St 0
                ((weigh (t) s
                  (resume (match t
                            ((Tree.Leaf v) v)
                            ((Tree.Node l r) 99)) s)))
                (+ (St.weigh (Tree.Leaf n)) (St.weigh (Tree.Node (Tree.Leaf 1) (Tree.Leaf 2))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 104 Int64)))
