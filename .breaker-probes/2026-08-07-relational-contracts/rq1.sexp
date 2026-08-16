(case "rq1 a RELATIONAL @requires over TWO heap params (lengths must agree) enforces at runtime"
  (input  (do
            (@ (requires (= (List.len xs) (List.len ys)))
              (def (zip-sum (: xs (List Int64)) (: ys (List Int64)) (: i Int64) (: acc Int64))
                (if (>= i (List.len xs)) acc
                    (zip-sum xs ys (+ i 1)
                      (+ acc (* (Option.expect (List.at xs i) "x")
                                (Option.expect (List.at ys i) "y")))))))
            (def (main (: n Int64))
              (zip-sum (list 1 2 3) (if (> n 0) (list 4 5 6) (list 4 5)) 0 0))
            (export main)))
  (call   main (: 1 Int64)) (output (: 32 Int64))
  (call   main (: 0 Int64)) (trap "unreachable"))
