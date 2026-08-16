(case "ae12 an IF-expression as an argument whose CONDITION draws — the taken branch decides whether a second draw fires before the next arg"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (tens (: a Int64) (: b Int64)) (+ (* 10 a) b))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (tens (if (> (E.next) 0) (E.next) 100) (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 67 Int64))
  (call   main (: -2 Int64)) (output (: 999 Int64)))
