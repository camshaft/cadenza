(case "inv1 an @invariant newtype REBUILT through a chain of constructor calls re-establishes each time"
  (input  (do
            (@ (invariant (< 0 (List.len self))) (type NEList (Mk (List Int64))))
            (def (grow (: ne NEList) (: v Int64))
              (match ne (((. NEList Mk) xs) (NEList.Mk (List.push xs v)))))
            (def (main (: k Int64))
              (let ((base (NEList.Mk (list k))))
                (let ((grown (grow (grow base 10) 20)))
                  (match grown (((. NEList Mk) xs) (+ (List.len xs)
                    (* 10 (match base (((. NEList Mk) b) (List.len b))))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 13 Int64)))
