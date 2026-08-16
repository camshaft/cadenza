(case "inv3 shrinking an @invariant newtype to EMPTY traps at the rebuild"
  (input  (do
            (@ (invariant (< 0 (List.len self))) (type NEList (Mk (List Int64))))
            (def (drop-first (: ne NEList))
              (match ne (((. NEList Mk) xs)
                (NEList.Mk (match xs ((list _h .. t) t) (_ (list)))))))
            (def (main (: k Int64))
              (let ((one (NEList.Mk (list k))))
                (match (drop-first one) (((. NEList Mk) xs) (List.len xs)))))
            (export main)))
  (call   main (: 5 Int64)) (trap "unreachable"))
