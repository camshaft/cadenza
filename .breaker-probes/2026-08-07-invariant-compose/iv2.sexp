(case "iv2 invariant-guarded values as CHAMP SET elements dedupe by payload content"
  (input  (do
            (@ (invariant (> self 0)) (type Pos (Mk Int64)))
            (def (main (: n Int64))
              (Set.len (Set.of (list (Pos.Mk n) (Pos.Mk 5) (Pos.Mk (+ n 4))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2 Int64))
  (call   main (: 1 Int64)) (output (: 2 Int64)))
