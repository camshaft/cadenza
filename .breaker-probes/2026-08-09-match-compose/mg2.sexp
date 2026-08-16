(case "mg2 a guard performing arithmetic on TWO pattern binders from a nested destructure"
  (input  (do
            (def (main (: a Int64) (: b Int64))
              (match (tuple (tuple a b) (+ a b))
                ((tuple (tuple x y) s) (if (= (+ x y) s) (* s 10) -1))))
            (export main)))
  (call   main (: 3 Int64) (: 4 Int64)) (output (: 70 Int64)))
