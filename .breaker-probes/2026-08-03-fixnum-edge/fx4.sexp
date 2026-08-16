(case "fx4 a Bytes value with fixnum-boundary CONTENT as a Map key (heap-payload boundary crossing)"
  (input  (do
            (def (main (: k Int64))
              (let ((key (list (+ 536870911 k) (- -536870912 k))))
                (match (Map.lookup (Map.insert Map.empty (list 536870912 -536870913) 9) key)
                  ((Some v) v) ((None _u) -1))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 9 Int64)))
