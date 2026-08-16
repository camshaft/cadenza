(case "fx2 the NEGATIVE fixnum boundary: -2^29-1 computed vs literal"
  (input  (do
            (def (main (: k Int64))
              (let ((boxed (- -536870912 k)))
                (+ (* 10 (if (= boxed -536870913) 1 0))
                   (Set.len (Set.of (list boxed -536870913))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 11 Int64)))
