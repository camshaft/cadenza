(case "iv3 an invariant payload built by a RECURSIVE producer checks once at the construct site"
  (input  (do
            (@ (invariant (< 0 (List.len self))) (type NEList (Mk (List Int64))))
            (def (build (: i Int64) (: acc (List Int64)))
              (if (= i 0) acc (build (- i 1) (List.push acc i))))
            (def (main (: n Int64))
              (match (NEList.Mk (build n (list)))
                (((. NEList Mk) ys) (List.len ys))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 40 Int64))
  (call   main (: 0 Int64)) (trap "unreachable"))
