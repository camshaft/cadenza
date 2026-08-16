(case "iv1 an invariant-guarded newtype value derived VIA ROW-OP-like update reconstruction re-checks"
  (input  (do
            (@ (invariant (< 0 (List.len self))) (type NEList (Mk (List Int64))))
            (def (shrink-to (: ne NEList) (: keep Int64))
              (match ne
                (((. NEList Mk) xs)
                  (NEList.Mk (if (> keep 0) (list (Option.expect (List.at xs 0) "hd")) (list))))))
            (def (main (: n Int64))
              (match (shrink-to (NEList.Mk (list 7 8 9)) n)
                (((. NEList Mk) ys) (List.len ys))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 1 Int64))
  (call   main (: 0 Int64)) (trap "unreachable"))
