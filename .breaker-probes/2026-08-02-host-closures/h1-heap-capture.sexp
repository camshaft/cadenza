(case "h1 a host-called closure capturing a HEAP list reads it per call (repeatable borrow)"
  (input  (do
            (def (make (: k Int64))
              (let ((xs (list k (+ k 1) (+ k 2))))
                (fn ((: i Int64))
                  (match (List.at xs i) ((Some v) v) ((None _u) -1)))))
            (export make)))
  (call   make (: 10 Int64) (: 2 Int64))
  (output (: 12 Int64)))
