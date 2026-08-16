(case "a host-crossing closure capturing a list ALSO stored in a tuple keeps both routes callable"
  (input  (do
            (def (main)
              (let ((xs (list 2 4 6)))
                (let ((f (fn ((: i Int64)) (match (List.at xs i) ((Some v) v) ((None u) -1)))))
                  (let ((t (tuple f 9)))
                    (. t 0)))))
            (export main)))
  (call   main (: 1 Int64))
  (output (: 4 Int64)))
