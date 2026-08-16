(case "cc8 a closure carried in a TUPLE beside a scalar — destructured in one match and applied around advancing draws"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (match (tuple (fn ((: x Int64)) (* x n)) 7)
                  ((tuple f c) (+ (f (St.next)) (+ c (f (St.next))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 62 Int64))
  (call   main (: 2 Int64)) (output (: 17 Int64)))
